//! Port of `src/gates/implement.ts` (`implementGate`, `scopeCheck`,
//! `commandCheck`). Ported in wave 3 (see `docs/rust-port.md`).
//!
//! IMPLEMENT gate - deterministic:
//!  - the working diff is non-empty
//!  - every touched file is covered by a `plan.files` glob (scope discipline)
//!  - build passes (if a build command is configured)
//!  - lint passes (if a lint command is configured)
//!
//! Changes under `.gate/` (run bookkeeping) are never counted as code
//! touched.
//!
//! Targets (Milestone 3): with no `targets:` configured, or none affected by
//! the touched files, `resolve_phase_targets` collapses to a single bare
//! entry using the top-level commands - byte-identical to pre-targets
//! behavior, including the check names. Once >=1 named target is affected,
//! each target gets its own `implement.build[name]` / `implement.lint[name]`
//! checks using that target's effective commands.

use crate::artifacts::plan::parse_plan_file;
use crate::core::exec::run_command;
use crate::core::git::{changed_files, is_gate_bookkeeping, is_git_repo};
use crate::core::glob::matches_any;
use crate::core::paths::run_paths;
use crate::core::state_machine::Phase;
use crate::core::targets::{check_name, resolve_phase_targets};
use crate::core::trust::is_commands_trusted;
use crate::gates::plan_drift::plan_drift_check;
use crate::gates::types::{fail, fail_untrusted, pass, result, Check, GateContext, GateResult};

pub fn implement_gate(ctx: &GateContext) -> GateResult {
    let mut checks: Vec<Check> = Vec::new();
    if let Some(drift) = plan_drift_check(ctx, "implement.plan-drift") {
        checks.push(drift);
    }

    if !is_git_repo(&ctx.root) {
        checks.push(fail(
            "implement.git",
            "not a git repository - cannot measure the diff",
        ));
        return result(Phase::Implement, checks);
    }

    let touched: Vec<String> = changed_files(&ctx.root, ctx.run.base_ref.as_deref())
        .into_iter()
        .filter(|f| !is_gate_bookkeeping(&ctx.root, f))
        .collect();

    checks.push(if !touched.is_empty() {
        pass(
            "implement.diff",
            format!("{} file(s) changed", touched.len()),
        )
    } else {
        fail(
            "implement.diff",
            "no code changes detected since the run started",
        )
    });

    checks.extend(scope_check("implement.scope", ctx, &touched));

    let trusted = is_commands_trusted(&ctx.root);
    for t in resolve_phase_targets(&ctx.config, ctx.run.target_override.as_deref(), &touched) {
        checks.push(command_check(
            &check_name("implement.build", t.target.as_deref()),
            t.commands.build.as_deref(),
            &ctx.root,
            "build",
            trusted,
        ));
        checks.push(command_check(
            &check_name("implement.lint", t.target.as_deref()),
            t.commands.lint.as_deref(),
            &ctx.root,
            "lint",
            trusted,
        ));
    }

    result(Phase::Implement, checks)
}

/// Scope discipline: every touched file (outside `.gate/`) must be covered
/// by a `plan.files` glob, or by `config.scope_ignore` (Milestone 5) -
/// environment cruft (a node compile cache, a go build dir, coverage
/// output) that isn't part of the plan but shouldn't hard-block the gate
/// either. Shared by the IMPLEMENT and DEBUG gates, both of which produce a
/// diff that must stay within the declared plan.
///
/// `scope_ignore` only takes effect when the commands block is trusted (it
/// rides in the same TOFU hash) - an untrusted edit to it never silently
/// widens what the scope check accepts. A match is always surfaced as its
/// own info-level check line, never folded silently into a passing result.
pub fn scope_check(name: &str, ctx: &GateContext, touched: &[String]) -> Vec<Check> {
    let plan = parse_plan_file(&run_paths(&ctx.root, &ctx.run.id).plan).plan;
    let declared: Vec<String> = plan.map(|p| p.files).unwrap_or_default();
    let ignore_globs = &ctx.config.scope_ignore;
    let trusted = is_commands_trusted(&ctx.root);

    let out_of_plan: Vec<&String> = touched
        .iter()
        .filter(|f| !matches_any(f, &declared))
        .collect();
    let ignored: Vec<&String> = if trusted {
        out_of_plan
            .iter()
            .filter(|f| matches_any(f, ignore_globs))
            .copied()
            .collect()
    } else {
        Vec::new()
    };
    let undeclared: Vec<&String> = out_of_plan
        .iter()
        .filter(|f| !ignored.contains(f))
        .copied()
        .collect();

    let mut checks: Vec<Check> = Vec::new();
    if undeclared.is_empty() {
        // Truthful even when some touched files weren't declared but
        // matched scope_ignore instead - "all ...
        // declared in plan.md" was misleading when `ignored` was the reason
        // some passed.
        let detail = if !ignored.is_empty() {
            "all touched files are declared in plan.md or matched scope_ignore"
        } else {
            "all touched files are declared in plan.md"
        };
        checks.push(pass(name, detail));
    } else {
        let untrusted_hint = if !trusted
            && !ignore_globs.is_empty()
            && out_of_plan.iter().any(|f| matches_any(f, ignore_globs))
        {
            " (scope_ignore is configured but untrusted - run `gate trust` to apply it)"
        } else {
            ""
        };
        let list = undeclared
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        checks.push(fail(
            name,
            format!("files touched but not declared in plan.md (add them or narrow scope): {list}{untrusted_hint}"),
        ));
    }
    if !ignored.is_empty() {
        let list = ignored
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        checks.push(pass(
            format!("{name}.ignored"),
            format!("matched scope_ignore, excluded from the scope check: {list}"),
        ));
    }
    checks
}

/// Runs a configured command; a missing command passes with a skip note.
/// When a command is set but the commands block is untrusted, it fails
/// *without running* - untrusted config never spawns a process.
pub fn command_check(
    name: &str,
    command: Option<&str>,
    cwd: &std::path::Path,
    label: &str,
    trusted: bool,
) -> Check {
    let Some(command) = command else {
        return pass(name, format!("no {label} command configured (skipped)"));
    };
    if !trusted {
        return fail_untrusted(
            name,
            format!(
                "{label} command not trusted - trust is per machine, per checkout; review .gate/config.yml and run `gate trust`"
            ),
        );
    }
    let res = run_command(command, cwd.to_str().unwrap_or("."), None);
    if res.code == 0 {
        return pass(name, format!("{label} passed"));
    }
    let source = if !res.stderr.is_empty() {
        &res.stderr
    } else {
        &res.stdout
    };
    let lines: Vec<&str> = source.trim().split('\n').collect();
    let start = lines.len().saturating_sub(3);
    let tail = lines[start..].join(" \u{23ce} ");
    fail(name, format!("{label} failed (exit {}): {tail}", res.code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::GateConfig;
    use crate::core::run::{new_run, NewRunParams};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command as StdCommand;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-implement-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_git(root: &std::path::Path, args: &[&str]) {
        assert!(StdCommand::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap()
            .success());
    }

    fn write_file(root: &std::path::Path, rel: &str, content: &str) {
        let abs = root.join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, content).unwrap();
    }

    fn make_repo(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = tmp_dir(name);
        run_git(&dir, &["init", "-q"]);
        run_git(&dir, &["config", "user.email", "t@t.co"]);
        run_git(&dir, &["config", "user.name", "t"]);
        run_git(&dir, &["config", "commit.gpgsign", "false"]);
        write_file(&dir, "README.md", "seed\n");
        for (path, content) in files {
            write_file(&dir, path, content);
        }
        run_git(&dir, &["add", "-A"]);
        run_git(&dir, &["commit", "-qm", "init"]);
        dir
    }

    fn head_sha(root: &std::path::Path) -> String {
        let out = StdCommand::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    const GOOD_PLAN: &str = "---\ngoal: Add greet\nfiles:\n  - greet.js\n  - test.js\ncriteria:\n  - id: c1\n    text: \"greets by name\"\n    verify: \"test: greets by name\"\n---\n# Plan";

    fn write_plan(root: &std::path::Path, content: &str) {
        write_file(root, ".gate/runs/r1/plan.md", content);
    }

    fn run_on(base_ref: Option<&str>) -> crate::core::run::Run {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: base_ref.map(String::from),
            session_id: None,
            target_override: None,
        });
        run.phase = Phase::Implement;
        run
    }

    fn check<'a>(res: &'a GateResult, name: &str) -> Option<&'a Check> {
        res.checks.iter().find(|c| c.name == name)
    }

    #[test]
    fn fails_with_no_diff() {
        let root = make_repo("no-diff", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config: GateConfig::default(),
        };
        let res = implement_gate(&ctx);
        assert!(!check(&res, "implement.diff").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn passes_with_an_in_scope_change() {
        let root = make_repo("in-scope", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        write_file(
            &root,
            "greet.js",
            "module.exports.greet = (n) => 'Hi ' + n;\n",
        );
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config: GateConfig::default(),
        };
        let res = implement_gate(&ctx);
        assert!(res.ok);
        assert!(check(&res, "implement.diff").unwrap().ok);
        assert!(check(&res, "implement.scope").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_an_out_of_scope_change() {
        let root = make_repo("out-of-scope", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        write_file(&root, "greet.js", "x\n");
        write_file(&root, "secret.js", "y\n");
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config: GateConfig::default(),
        };
        let res = implement_gate(&ctx);
        assert!(!check(&res, "implement.scope").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ignores_gate_bookkeeping_when_measuring_the_diff() {
        let root = make_repo("bookkeeping", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        write_file(&root, ".gate/runs/r1/worklog.md", "note\n");
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config: GateConfig::default(),
        };
        let res = implement_gate(&ctx);
        assert!(!check(&res, "implement.diff").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    fn write_config(root: &std::path::Path, yaml_body: &str, trust: bool) -> GateConfig {
        fs::create_dir_all(root.join(".gate")).unwrap();
        fs::write(root.join(".gate/config.yml"), yaml_body).unwrap();
        if trust {
            crate::core::trust::write_trust(root, None).unwrap();
        }
        crate::core::config::load_config(root).unwrap()
    }

    #[test]
    fn fails_when_a_trusted_build_command_fails() {
        let root = make_repo("build-fail", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        write_file(&root, "greet.js", "x\n");
        let config = write_config(&root, "commands:\n  build: \"exit 3\"\n", true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config,
        };
        let res = implement_gate(&ctx);
        assert!(!check(&res, "implement.build").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_run_an_untrusted_build_command() {
        let root = make_repo("build-untrusted", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        write_file(&root, "greet.js", "x\n");
        let config = write_config(&root, "commands:\n  build: \"touch ran.txt\"\n", false);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config,
        };
        let res = implement_gate(&ctx);
        assert!(!check(&res, "implement.build").unwrap().ok);
        assert!(!root.join("ran.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scope_ignore_false_noise_goes_green_instead_of_hard_blocking() {
        let root = make_repo("scope-ignore-noise", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        write_file(&root, "greet.js", "x\n");
        write_file(&root, ".cache/build-info.json", "{}\n");
        let config = write_config(
            &root,
            "commands: {}\nscope_ignore:\n  - \".cache/**\"\n",
            true,
        );
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config,
        };
        let res = implement_gate(&ctx);
        assert!(res.ok);
        assert!(check(&res, "implement.scope").unwrap().ok);
        let ignored = check(&res, "implement.scope.ignored").unwrap();
        assert!(ignored.ok);
        assert!(ignored.detail.contains(".cache/build-info.json"));
        let scope_detail = &check(&res, "implement.scope").unwrap().detail;
        assert!(scope_detail.contains("scope_ignore"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_real_undeclared_file_still_fails_even_with_scope_ignore_configured() {
        let root = make_repo("scope-ignore-real-undeclared", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        write_file(&root, "greet.js", "x\n");
        write_file(&root, "secret.js", "y\n");
        let config = write_config(
            &root,
            "commands: {}\nscope_ignore:\n  - \".cache/**\"\n",
            true,
        );
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config,
        };
        let res = implement_gate(&ctx);
        let failing = check(&res, "implement.scope").unwrap();
        assert!(!failing.ok);
        assert!(failing.detail.contains("secret.js"));
        assert!(check(&res, "implement.scope.ignored").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_untrusted_scope_ignore_edit_refuses_to_apply() {
        let root = make_repo("scope-ignore-untrusted", &[("greet.js", "")]);
        let base = head_sha(&root);
        write_plan(&root, GOOD_PLAN);
        write_file(&root, "greet.js", "x\n");
        write_file(&root, ".cache/build-info.json", "{}\n");
        write_config(&root, "commands: {}\nscope_ignore: []\n", true);
        // Edit the ignore glob in directly, without re-running `gate trust`.
        fs::write(
            root.join(".gate/config.yml"),
            "commands: {}\nscope_ignore:\n  - \".cache/**\"\n",
        )
        .unwrap();
        let config = crate::core::config::load_config(&root).unwrap();
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(Some(&base)),
            config,
        };
        let res = implement_gate(&ctx);
        let failing = check(&res, "implement.scope").unwrap();
        assert!(!failing.ok);
        assert!(failing.detail.contains(".cache/build-info.json"));
        assert!(failing.detail.contains("untrusted"));
        assert!(check(&res, "implement.scope.ignored").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn plan_drift_is_silent_when_never_approved_or_unchanged() {
        let root = make_repo("drift-silent", &[]);
        write_plan(&root, GOOD_PLAN);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(None),
            config: GateConfig::default(),
        };
        let res = implement_gate(&ctx);
        assert!(check(&res, "implement.plan-drift").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn plan_drift_fails_once_the_plan_changed_after_approval() {
        let root = make_repo("drift-fails", &[]);
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on(None);
        run.approval = Some(crate::core::run::Approval {
            by: None,
            at: crate::core::run::now_iso(),
            reason: None,
            plan_hash: crate::artifacts::plan::hash_plan(GOOD_PLAN),
        });
        write_plan(
            &root,
            &format!("{GOOD_PLAN}\nscope widened after approval\n"),
        );
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = implement_gate(&ctx);
        assert!(!check(&res, "implement.plan-drift").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn command_check_reports_no_command_configured_when_absent() {
        let check = command_check("x", None, std::path::Path::new("."), "build", true);
        assert!(check.ok);
        assert!(check.detail.contains("no build command configured"));
    }

    #[test]
    fn command_check_fails_without_running_when_untrusted() {
        let dir = tmp_dir("command-check-untrusted");
        let check = command_check("x", Some("touch ran.txt"), &dir, "build", false);
        assert!(!check.ok);
        assert!(!dir.join("ran.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
