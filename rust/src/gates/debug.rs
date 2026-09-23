//! Port of `src/gates/debug.ts` (`debugGate`). Ported in wave 3 (see
//! `docs/rust-port.md`).
//!
//! DEBUG gate - deterministic proof that a bug was diagnosed, not guessed
//! at:
//!  - debug-log.md exists and follows the protocol schema
//!  - reproduction is attested (`reproduced: true` - an agent claim, not
//!    proof; the mechanical proof is the triggering test below)
//!  - at least one *complete* cycle (hypothesis -> prediction -> experiment
//!    -> observation -> conclusion) is recorded
//!  - the fix stayed within the declared plan scope
//!  - the triggering test is green AND the whole suite is green (no
//!    regressions)
//!
//! The protocol itself - asking good questions, ruling causes out - is
//! judgment and lives in the DEBUG playbook. The gate only checks that the
//! ritual happened and the evidence is real.

use std::collections::HashMap;
use std::path::Path;

use crate::artifacts::debug_log::{is_cycle_complete, parse_debug_log_file};
use crate::core::exec::run_command;
use crate::core::git::{changed_files, is_gate_bookkeeping};
use crate::core::paths::run_paths;
use crate::core::state_machine::Phase;
use crate::core::targets::{check_name, resolve_phase_targets, ResolvedTarget};
use crate::core::trust::is_commands_trusted;
use crate::gates::implement::scope_check;
use crate::gates::plan_drift::plan_drift_check;
use crate::gates::test::{load_report, stat_report, target_report_path};
use crate::gates::test_report::{NormalizedReport, NormalizedTest, TestStatus};
use crate::gates::types::{fail, fail_untrusted, pass, result, Check, GateContext, GateResult};

pub fn debug_gate(ctx: &GateContext) -> GateResult {
    let mut checks: Vec<Check> = Vec::new();
    if let Some(drift) = plan_drift_check(ctx, "debug.plan-drift") {
        checks.push(drift);
    }
    let paths = run_paths(&ctx.root, &ctx.run.id);

    let parsed = parse_debug_log_file(&paths.debug_log);
    let Some(log) = parsed.log else {
        checks.push(fail(
            "debug.log",
            format!("debug-log.md invalid: {}", parsed.errors.join("; ")),
        ));
        return result(Phase::Debug, checks);
    };
    checks.push(pass(
        "debug.log",
        "debug-log.md matches the protocol schema",
    ));

    checks.push(if log.reproduced {
        pass("debug.reproduced", "bug reproduced before diagnosing")
    } else {
        fail(
            "debug.reproduced",
            "set `reproduced: true` only after you have reproduced the failure",
        )
    });

    let complete = log.cycles.iter().filter(|c| is_cycle_complete(c)).count();
    checks.push(if complete >= 1 {
        pass("debug.cycle", format!("{complete} complete debugging cycle(s)"))
    } else {
        fail(
            "debug.cycle",
            "no complete cycle - each needs hypothesis, prediction, experiment, observation, conclusion and status: complete",
        )
    });

    let touched: Vec<String> = changed_files(&ctx.root, ctx.run.base_ref.as_deref())
        .into_iter()
        .filter(|f| !is_gate_bookkeeping(&ctx.root, f))
        .collect();
    checks.extend(scope_check("debug.scope", ctx, &touched));

    let resolved = resolve_phase_targets(&ctx.config, ctx.run.target_override.as_deref(), &touched);
    if resolved.len() == 1 && resolved[0].target.is_none() {
        checks.push(triggering_test_check(
            ctx,
            &log.triggering_test,
            &paths.test_report,
            &resolved[0],
        ));
    } else {
        checks.extend(targeted_triggering_test_checks(
            ctx,
            &log.triggering_test,
            &paths.test_report,
            &resolved,
        ));
    }

    result(Phase::Debug, checks)
}

fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.trim().split('\n').collect();
    let start = lines.len().saturating_sub(3);
    lines[start..].join(" \u{23ce} ")
}

/// Runs the test suite and asserts the triggering test is now green with no
/// regressions: the command exits 0 (whole suite green) and the report
/// contains a passing test whose name matches the triggering test. Mirrors
/// the TEST gate's trust discipline - untrusted config never spawns a
/// process - and its evidence discipline: the report must come from the run
/// Gate just executed, and its absence fails closed (a bugfix whose fix
/// cannot be named is not verified).
///
/// Bare/legacy case only (no targets configured, or none affected) -
/// reproduces the pre-targets check byte-for-byte, including the
/// "debug.test-green" name and report path.
fn triggering_test_check(
    ctx: &GateContext,
    triggering: &str,
    report_path: &Path,
    t: &ResolvedTarget,
) -> Check {
    let name = check_name("debug.test-green", t.target.as_deref());
    let Some(test_cmd) = &t.commands.test else {
        return fail(name, "no test command configured (commands.test)");
    };
    if !is_commands_trusted(&ctx.root) {
        return fail_untrusted(
            name,
            "test command not trusted - trust is per machine, per checkout; review .gate/config.yml and run `gate trust`",
        );
    }
    let dir = run_paths(&ctx.root, &ctx.run.id).dir;
    let path = target_report_path(report_path, t.target.as_deref());
    let report_before = stat_report(&path);
    let mut env = HashMap::new();
    env.insert(
        "GATE_RUN_DIR".to_string(),
        dir.to_string_lossy().into_owned(),
    );
    env.insert(
        "GATE_TEST_REPORT".to_string(),
        path.to_string_lossy().into_owned(),
    );
    let run = run_command(test_cmd, ctx.root.to_str().unwrap_or("."), Some(&env));
    if run.code != 0 {
        let source = if !run.stderr.is_empty() {
            &run.stderr
        } else {
            &run.stdout
        };
        return fail(
            name,
            format!(
                "suite still red (exit {}) - no regressions allowed: {}",
                run.code,
                tail(source)
            ),
        );
    }
    let Some(report) = load_report(&path, &run.stdout, report_before.as_ref()) else {
        return fail(
            name,
            "suite green, but no parseable report to confirm the triggering test - \
emit a JSON report on stdout or write it to $GATE_TEST_REPORT",
        );
    };
    let needle = triggering.to_lowercase();
    let hit = report
        .tests
        .iter()
        .any(|r| r.status == TestStatus::Passed && r.name.to_lowercase().contains(&needle));
    if hit {
        pass(
            name,
            format!("triggering test \"{triggering}\" passes; suite green"),
        )
    } else {
        fail(
            name,
            format!(
                "suite is green but no passing test matches the triggering test \"{triggering}\""
            ),
        )
    }
}

/// At least one named target is affected. Each target's suite must be green -
/// checked per target so one target's regression doesn't hide behind
/// another's pass - but the triggering test's name is a property of the
/// bugfix as a whole, not of any single stack, so it's asserted once
/// against the COMBINED report across every affected target (mirrors the
/// TEST gate's aggregate test.criteria / test.no-skips over
/// `run_targeted_gate`'s combined report).
fn targeted_triggering_test_checks(
    ctx: &GateContext,
    triggering: &str,
    report_path: &Path,
    resolved: &[ResolvedTarget],
) -> Vec<Check> {
    let mut checks: Vec<Check> = Vec::new();
    let mut reports: Vec<NormalizedReport> = Vec::new();
    let dir = run_paths(&ctx.root, &ctx.run.id).dir;
    let trusted = is_commands_trusted(&ctx.root);

    for t in resolved {
        let name = check_name("debug.test-green", t.target.as_deref());
        let Some(test_cmd) = &t.commands.test else {
            checks.push(fail(name, "no test command configured (commands.test)"));
            continue;
        };
        if !trusted {
            checks.push(fail_untrusted(
                name,
                "test command not trusted - trust is per machine, per checkout; review .gate/config.yml and run `gate trust`",
            ));
            continue;
        }
        let path = target_report_path(report_path, t.target.as_deref());
        let report_before = stat_report(&path);
        let mut env = HashMap::new();
        env.insert(
            "GATE_RUN_DIR".to_string(),
            dir.to_string_lossy().into_owned(),
        );
        env.insert(
            "GATE_TEST_REPORT".to_string(),
            path.to_string_lossy().into_owned(),
        );
        let run = run_command(test_cmd, ctx.root.to_str().unwrap_or("."), Some(&env));
        if run.code != 0 {
            let source = if !run.stderr.is_empty() {
                &run.stderr
            } else {
                &run.stdout
            };
            checks.push(fail(
                name,
                format!(
                    "suite still red (exit {}) - no regressions allowed: {}",
                    run.code,
                    tail(source)
                ),
            ));
            continue;
        }
        checks.push(pass(name, "suite green (exit 0)"));
        match load_report(&path, &run.stdout, report_before.as_ref()) {
            Some(r) => reports.push(r),
            None => checks.push(fail(
                check_name("debug.report", t.target.as_deref()),
                "suite green, but no parseable report to confirm the triggering test - \
emit a JSON report on stdout or write it to $GATE_TEST_REPORT",
            )),
        }
    }

    let combined: Option<NormalizedReport> = if !reports.is_empty() {
        let tests: Vec<NormalizedTest> = reports
            .iter()
            .flat_map(|r| r.tests.iter().cloned())
            .collect();
        let skipped: usize = reports.iter().map(|r| r.skipped).sum();
        Some(NormalizedReport { tests, skipped })
    } else {
        None
    };

    let needle = triggering.to_lowercase();
    match &combined {
        None => checks.push(fail(
            "debug.triggering-test",
            "no parseable test report across the affected targets to confirm the triggering test",
        )),
        Some(combined) => {
            let hit = combined
                .tests
                .iter()
                .any(|r| r.status == TestStatus::Passed && r.name.to_lowercase().contains(&needle));
            checks.push(if hit {
                pass(
                    "debug.triggering-test",
                    format!("triggering test \"{triggering}\" passes across the affected targets"),
                )
            } else {
                fail(
                    "debug.triggering-test",
                    format!("no passing test across the affected targets matches the triggering test \"{triggering}\""),
                )
            });
        }
    }

    checks
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
        let dir = crate::core::testutil::unique_temp_dir(&format!("debug-rs-{name}"));
        dir
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let abs = root.join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, content).unwrap();
    }

    fn make_repo(name: &str) -> PathBuf {
        let dir = tmp_dir(name);
        let git = |args: &[&str]| {
            assert!(StdCommand::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.co"]);
        git(&["config", "user.name", "t"]);
        git(&["config", "commit.gpgsign", "false"]);
        write_file(&dir, "README.md", "seed\n");
        git(&["add", "-A"]);
        git(&["commit", "-qm", "init"]);
        dir
    }

    const GOOD_PLAN: &str = "---\ngoal: Add greet\nfiles:\n  - greet.js\n  - test.js\ncriteria:\n  - id: c1\n    text: \"greets by name\"\n    verify: \"test: greets by name\"\n---\n# Plan";

    const GOOD_DEBUG: &str = "---\ntriggering_test: \"greets by name\"\nreproduced: true\ncycles:\n  - hypothesis: \"units mismatch\"\n    prediction: \"logs show a 1000x gap\"\n    experiment: \"logged both sides\"\n    observation: \"off by 1000x\"\n    conclusion: \"confirmed units bug\"\n    status: complete\n---\n# Debug log";

    fn write_plan(root: &Path, content: &str) {
        write_file(root, ".gate/runs/r1/plan.md", content);
    }
    fn write_debug_log(root: &Path, content: &str) {
        write_file(root, ".gate/runs/r1/debug-log.md", content);
    }

    fn run_on() -> crate::core::run::Run {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.phase = Phase::Debug;
        run
    }

    fn yaml_quote(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }

    fn write_config(root: &Path, yaml_body: &str, trust: bool) -> GateConfig {
        fs::create_dir_all(root.join(".gate")).unwrap();
        fs::write(root.join(".gate/config.yml"), yaml_body).unwrap();
        if trust {
            crate::core::trust::write_trust(root, None).unwrap();
        }
        crate::core::config::load_config(root).unwrap()
    }

    fn write_test_command(root: &Path, cmd: &str, trust: bool) -> GateConfig {
        write_config(
            root,
            &format!("commands:\n  test: {}\n", yaml_quote(cmd)),
            trust,
        )
    }

    fn node_script_cmd(root: &Path, name: &str, script: &str) -> String {
        let path = root.join(name);
        fs::write(&path, script).unwrap();
        format!("node {}", path.display())
    }

    fn green_suite_cmd(root: &Path) -> String {
        node_script_cmd(
            root,
            "test.js",
            "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'}]}));\n",
        )
    }

    fn check<'a>(res: &'a GateResult, name: &str) -> Option<&'a Check> {
        res.checks.iter().find(|c| c.name == name)
    }

    #[test]
    fn passes_a_reproduced_bug_with_a_complete_cycle_and_a_green_triggering_test() {
        let root = make_repo("debug-pass");
        write_plan(&root, GOOD_PLAN);
        write_debug_log(&root, GOOD_DEBUG);
        let cmd = green_suite_cmd(&root);
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = debug_gate(&ctx);
        assert!(res.ok, "{:?}", res.checks);
        assert!(check(&res, "debug.reproduced").unwrap().ok);
        assert!(check(&res, "debug.cycle").unwrap().ok);
        assert!(check(&res, "debug.test-green").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_the_log_is_missing() {
        let root = make_repo("debug-missing-log");
        write_plan(&root, GOOD_PLAN);
        let cmd = green_suite_cmd(&root);
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = debug_gate(&ctx);
        assert!(!check(&res, "debug.log").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_the_bug_was_not_reproduced() {
        let root = make_repo("debug-not-reproduced");
        write_plan(&root, GOOD_PLAN);
        write_debug_log(
            &root,
            &GOOD_DEBUG.replace("reproduced: true", "reproduced: false"),
        );
        let cmd = green_suite_cmd(&root);
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = debug_gate(&ctx);
        assert!(!check(&res, "debug.reproduced").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_no_cycle_is_complete() {
        let root = make_repo("debug-no-complete-cycle");
        write_plan(&root, GOOD_PLAN);
        write_debug_log(
            &root,
            &GOOD_DEBUG.replace("status: complete", "status: in-progress"),
        );
        let cmd = green_suite_cmd(&root);
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = debug_gate(&ctx);
        assert!(!check(&res, "debug.cycle").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_the_suite_is_still_red() {
        let root = make_repo("debug-red-suite");
        write_plan(&root, GOOD_PLAN);
        write_debug_log(&root, GOOD_DEBUG);
        let cmd = node_script_cmd(&root, "test.js", "process.exit(1);\n");
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = debug_gate(&ctx);
        assert!(!check(&res, "debug.test-green").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_run_an_untrusted_test_command() {
        let root = make_repo("debug-untrusted");
        write_plan(&root, GOOD_PLAN);
        write_debug_log(&root, GOOD_DEBUG);
        let config = write_test_command(&root, "touch ran.txt", false);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = debug_gate(&ctx);
        assert!(!check(&res, "debug.test-green").unwrap().ok);
        assert!(!root.join("ran.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_closed_when_the_green_suite_emits_no_parseable_report() {
        let root = make_repo("debug-green-no-report");
        write_plan(&root, GOOD_PLAN);
        write_debug_log(&root, GOOD_DEBUG);
        let cmd = node_script_cmd(&root, "test.js", "process.exit(0);\n");
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = debug_gate(&ctx);
        assert!(!check(&res, "debug.test-green").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }
}
