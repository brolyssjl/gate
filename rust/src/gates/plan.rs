//! Port of `src/gates/plan.ts` (`planGate`). Ported in wave 3 (see
//! `docs/rust-port.md`).
//!
//! PLAN gate - all deterministic:
//!  - plan.md exists and parses against the schema
//!  - >=1 acceptance criterion, each with a declared verification method
//!  - approved via `gate approve`, bound to the current plan's content hash
//!  - when an SDD directory is detected and not disabled, the plan cites a
//!    spec path under it (advisory otherwise - see `spec_check`)
//!
//! Approval lives in run.json (written by `gate approve`), not in the
//! agent-editable plan.md, so the author can't self-approve; editing the
//! plan after approval voids it. Plan *quality* (are these the right
//! criteria?) is judgment and lives in the playbook, not here.

use std::collections::HashSet;
use std::path::Path;

use crate::artifacts::plan::{hash_plan_file, parse_plan_file, Plan};
use crate::core::config::GateConfig;
use crate::core::paths::run_paths;
use crate::core::state_machine::Phase;
use crate::gates::types::{fail, pass, result, Check, GateContext, GateResult};
use crate::integrations::advise::load_advise_report;
use crate::integrations::sdd_dir;

fn integration<'a>(config: &'a GateConfig, key: &str) -> Option<&'a str> {
    config
        .integrations
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub fn plan_gate(ctx: &GateContext) -> GateResult {
    let mut checks: Vec<Check> = Vec::new();
    let plan_path = run_paths(&ctx.root, &ctx.run.id).plan;
    let parsed = parse_plan_file(&plan_path);

    let Some(plan) = parsed.plan else {
        checks.push(fail(
            "plan.schema",
            format!("plan.md invalid: {}", parsed.errors.join("; ")),
        ));
        return result(Phase::Plan, checks);
    };

    checks.push(pass("plan.schema", "plan.md matches the required schema"));
    checks.push(if !plan.criteria.is_empty() {
        pass(
            "plan.criteria",
            format!("{} acceptance criteria", plan.criteria.len()),
        )
    } else {
        fail(
            "plan.criteria",
            "at least one acceptance criterion is required",
        )
    });
    // Every criterion carries a verify method (enforced by the parser), so
    // "checkable" is already satisfied when the schema is valid; surface it.
    checks.push(pass(
        "plan.checkable",
        "every criterion declares a verification method",
    ));
    if let Some(spec) = spec_check(ctx, &plan) {
        checks.push(spec);
    }
    if let Some(advise) = advise_check(ctx, &plan, &plan_path) {
        checks.push(advise);
    }
    checks.push(approval_check(ctx, &plan_path));
    result(Phase::Plan, checks)
}

/// `node:path.normalize` for a relative POSIX path: resolves `.`/`..`
/// segments (a leading `..` that would escape the start is kept, matching
/// Node), collapses repeated `/`.
fn normalize_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                if let Some(&last) = out.last() {
                    if last == ".." {
                        out.push("..");
                    } else {
                        out.pop();
                    }
                } else {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    if out.is_empty() {
        ".".to_string()
    } else {
        out.join("/")
    }
}

/// `node:path.relative(from, to)` for two relative POSIX paths (both
/// interpreted relative to the same, irrelevant cwd - which cancels out of
/// the computation, so it's never actually resolved here).
fn relative_path(from: &str, to: &str) -> String {
    let from_parts: Vec<&str> = from.split('/').filter(|s| !s.is_empty()).collect();
    let to_parts: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let mut i = 0;
    while i < from_parts.len() && i < to_parts.len() && from_parts[i] == to_parts[i] {
        i += 1;
    }
    let ups = from_parts.len() - i;
    let mut result: Vec<&str> = Vec::new();
    let up_dots: Vec<&str> = vec![".."; ups];
    result.extend(up_dots);
    result.extend(&to_parts[i..]);
    result.join("/")
}

/// SDD citation (Milestone 3, additive): fires only when an SDD directory is
/// detected on disk and `integrations.sdd` is not explicitly "off" -
/// presence plus opt-out, never a hard dependency on the framework being
/// installed. Not added to the checks at all otherwise, so the JSON schema
/// for a repo with no SDD framework stays byte-identical to before this
/// feature existed.
fn spec_check(ctx: &GateContext, plan: &Plan) -> Option<Check> {
    if integration(&ctx.config, "sdd") == Some("off") {
        return None;
    }
    let dir = sdd_dir(&ctx.root)?;

    let Some(spec) = &plan.spec else {
        return Some(fail(
            "plan.spec",
            format!("SDD detected ({dir}) - cite the spec path in plan.md's `spec:` field instead of restating it"),
        ));
    };
    // Normalize first (collapses "..") and check containment via a relative
    // path, not a string prefix - `openspec/../README.md` starts with the
    // string "openspec/" but normalizes to "README.md", outside `dir`
    // entirely. A naive `startsWith(dir + "/")` on the raw path is defeated
    // by exactly that "..".
    let normalized = normalize_path(spec.strip_prefix("./").unwrap_or(spec));
    let rel = relative_path(dir, &normalized);
    let contained = rel.is_empty() || (rel != ".." && !rel.starts_with("../"));
    if !contained {
        return Some(fail(
            "plan.spec",
            format!("spec path \"{spec}\" is not under the detected SDD dir \"{dir}\""),
        ));
    }
    if !ctx.root.join(&normalized).exists() {
        return Some(fail(
            "plan.spec",
            format!("spec path \"{spec}\" does not exist"),
        ));
    }
    Some(pass("plan.spec", format!("cites spec {normalized}")))
}

/// Advise consumption (Milestone 3, additive): fires only when an
/// `.agnosgram/` store is present and `integrations.agnosgram` is not
/// "off" - same presence + opt-out shape as `spec_check`. Gate never runs
/// `agnosgram advise` itself (advisory, never load-bearing); it only reads
/// whatever report is already on disk. A missing or unparseable report is
/// "no report": advisory pass with a hint, never a failure - a repo without
/// Agnosgram installed must behave exactly as before this feature existed.
fn advise_check(ctx: &GateContext, plan: &Plan, plan_path: &Path) -> Option<Check> {
    if integration(&ctx.config, "agnosgram") == Some("off") {
        return None;
    }
    if !ctx.root.join(".agnosgram").exists() {
        return None;
    }

    let Some(report) = load_advise_report(&plan_path.to_string_lossy()) else {
        return Some(pass(
            "plan.advise",
            "no agnosgram advise report found - advisory only; run `agnosgram advise` to check for contradictions",
        ));
    };
    let acknowledged: HashSet<&str> = plan.acknowledgments.iter().map(|s| s.as_str()).collect();
    let unacknowledged: Vec<_> = report
        .contradictions
        .iter()
        .filter(|c| !acknowledged.contains(c.record_id.as_str()))
        .collect();
    if !unacknowledged.is_empty() {
        let list = unacknowledged
            .iter()
            .map(|c| format!("{} ({})", c.record_id, c.severity))
            .collect::<Vec<_>>()
            .join(", ");
        return Some(fail(
            "plan.advise",
            format!(
                "unacknowledged contradictions: {list} - resolve them, then list the ids under plan.md's `acknowledgments:`"
            ),
        ));
    }
    Some(pass(
        "plan.advise",
        if report.contradictions.is_empty() {
            "advise report is clear - no contradictions".to_string()
        } else {
            format!(
                "{} contradiction(s), all acknowledged",
                report.contradictions.len()
            )
        },
    ))
}

fn approval_check(ctx: &GateContext, plan_path: &Path) -> Check {
    let Some(approval) = &ctx.run.approval else {
        return fail(
            "plan.approved",
            "plan not approved - get sign-off, then run `gate approve`",
        );
    };
    let current_hash = hash_plan_file(plan_path);
    if current_hash.as_deref() != Some(approval.plan_hash.as_str()) {
        return fail(
            "plan.approved",
            "plan changed since approval - re-run `gate approve` on the current plan",
        );
    }
    pass(
        "plan.approved",
        match &approval.by {
            Some(by) => format!("plan approved by {by}"),
            None => "plan approved".to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::plan::hash_plan;
    use crate::core::run::{new_run, now_iso, Approval, NewRunParams};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command as StdCommand;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-plan-gate-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
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

    fn write_plan(root: &Path, content: &str) {
        write_file(root, ".gate/runs/r1/plan.md", content);
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
        run.phase = Phase::Plan;
        run
    }

    fn approve(root: &Path, run: &mut crate::core::run::Run) {
        let plan_path = run_paths(root, "r1").plan;
        let raw = fs::read_to_string(&plan_path).unwrap();
        run.approval = Some(Approval {
            by: None,
            at: now_iso(),
            reason: None,
            plan_hash: hash_plan(&raw),
        });
    }

    fn check<'a>(res: &'a GateResult, name: &str) -> Option<&'a Check> {
        res.checks.iter().find(|c| c.name == name)
    }

    fn write_config(root: &Path, yaml_body: &str) -> GateConfig {
        fs::create_dir_all(root.join(".gate")).unwrap();
        fs::write(root.join(".gate/config.yml"), yaml_body).unwrap();
        crate::core::config::load_config(root).unwrap()
    }

    #[test]
    fn passes_a_valid_approved_plan() {
        let root = make_repo("plan-pass");
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        assert!(plan_gate(&ctx).ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_the_plan_is_missing() {
        let root = make_repo("plan-missing");
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(!check(&res, "plan.schema").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_not_approved() {
        let root = make_repo("plan-not-approved");
        write_plan(&root, GOOD_PLAN);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(!check(&res, "plan.approved").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn voids_approval_when_the_plan_changes_after_approval() {
        let root = make_repo("plan-voided");
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on();
        approve(&root, &mut run);
        write_plan(&root, &format!("{GOOD_PLAN}\nedited after approval\n"));
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(!check(&res, "plan.approved").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_add_plan_spec_when_no_sdd_directory_is_detected() {
        let root = make_repo("plan-no-sdd");
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(check(&res, "plan.spec").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn requires_a_spec_citation_once_an_sdd_directory_is_detected() {
        let root = make_repo("plan-sdd-required");
        write_file(&root, "openspec/changes/x/spec.md", "# spec\n");
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(!check(&res, "plan.spec").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn passes_plan_spec_when_the_cited_path_exists_under_the_detected_sdd_dir() {
        let root = make_repo("plan-sdd-cited");
        write_file(&root, "openspec/changes/x/spec.md", "# spec\n");
        write_plan(
            &root,
            &GOOD_PLAN.replace(
                "goal: Add greet",
                "goal: Add greet\nspec: openspec/changes/x/spec.md",
            ),
        );
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(check(&res, "plan.spec").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_plan_spec_when_the_cited_path_is_outside_the_sdd_dir() {
        let root = make_repo("plan-sdd-outside");
        write_file(&root, "openspec/changes/x/spec.md", "# spec\n");
        write_file(&root, "elsewhere.md", "not a spec\n");
        write_plan(
            &root,
            &GOOD_PLAN.replace("goal: Add greet", "goal: Add greet\nspec: elsewhere.md"),
        );
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(!check(&res, "plan.spec").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_plan_spec_when_dot_dot_would_escape_the_sdd_dir_after_normalization() {
        let root = make_repo("plan-sdd-dotdot");
        write_file(&root, "openspec/changes/x/spec.md", "# spec\n");
        write_file(&root, "README.md", "not a spec\n");
        write_plan(
            &root,
            &GOOD_PLAN.replace(
                "goal: Add greet",
                "goal: Add greet\nspec: openspec/../README.md",
            ),
        );
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(!check(&res, "plan.spec").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skips_plan_spec_entirely_when_integrations_sdd_is_off() {
        let root = make_repo("plan-sdd-off");
        write_file(&root, "openspec/changes/x/spec.md", "# spec\n");
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on();
        approve(&root, &mut run);
        let config = write_config(&root, "integrations:\n  sdd: off\n");
        let ctx = GateContext {
            root: root.clone(),
            run,
            config,
        };
        let res = plan_gate(&ctx);
        assert!(check(&res, "plan.spec").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_add_plan_advise_without_an_agnosgram_store() {
        let root = make_repo("plan-advise-no-store");
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(check(&res, "plan.advise").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn passes_plan_advise_with_a_note_when_a_store_exists_but_no_report_was_generated() {
        let root = make_repo("plan-advise-no-report");
        write_file(&root, ".agnosgram/config.yml", "version: 1\n");
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(check(&res, "plan.advise").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_plan_advise_on_an_unacknowledged_contradiction() {
        let root = make_repo("plan-advise-unack");
        write_file(&root, ".agnosgram/config.yml", "version: 1\n");
        write_plan(&root, GOOD_PLAN);
        write_file(
            &root,
            ".gate/runs/r1/plan.md.advise.json",
            r#"{"agnosgram_advise":1,"plan":".gate/runs/r1/plan.md","generated":"2026-07-27","checked_ids":["LES-002"],"contradictions":[{"record_id":"LES-002","severity":"blocker","kind":"empirical"}],"clear":false}"#,
        );
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(!check(&res, "plan.advise").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn passes_plan_advise_once_every_contradiction_is_acknowledged() {
        let root = make_repo("plan-advise-ack");
        write_file(&root, ".agnosgram/config.yml", "version: 1\n");
        write_plan(
            &root,
            &GOOD_PLAN.replace(
                "goal: Add greet",
                "goal: Add greet\nacknowledgments: [LES-002]",
            ),
        );
        write_file(
            &root,
            ".gate/runs/r1/plan.md.advise.json",
            r#"{"agnosgram_advise":1,"plan":".gate/runs/r1/plan.md","generated":"2026-07-27","checked_ids":["LES-002"],"contradictions":[{"record_id":"LES-002","severity":"blocker","kind":"empirical"}],"clear":false}"#,
        );
        let mut run = run_on();
        approve(&root, &mut run);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = plan_gate(&ctx);
        assert!(check(&res, "plan.advise").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skips_plan_advise_entirely_when_integrations_agnosgram_is_off() {
        let root = make_repo("plan-advise-off");
        write_file(&root, ".agnosgram/config.yml", "version: 1\n");
        write_plan(&root, GOOD_PLAN);
        let mut run = run_on();
        approve(&root, &mut run);
        let config = write_config(&root, "integrations:\n  agnosgram: off\n");
        let ctx = GateContext {
            root: root.clone(),
            run,
            config,
        };
        let res = plan_gate(&ctx);
        assert!(check(&res, "plan.advise").is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn normalize_path_collapses_dot_dot_segments() {
        assert_eq!(normalize_path("openspec/../README.md"), "README.md");
        assert_eq!(normalize_path("a/./b"), "a/b");
        assert_eq!(normalize_path("a/b/../../c"), "c");
    }

    #[test]
    fn relative_path_computes_the_expected_relative_form() {
        assert_eq!(
            relative_path("openspec", "openspec/changes/x/spec.md"),
            "changes/x/spec.md"
        );
        assert_eq!(relative_path("openspec", "README.md"), "../README.md");
        assert_eq!(relative_path("a", "a"), "");
    }
}
