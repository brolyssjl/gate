//! Port of `src/gates/review.ts` (`reviewGate`). Ported in wave 3 (see
//! `docs/rust-port.md`).
//!
//! REVIEW gate - deterministic checks over a fresh, self-contained review:
//!  - a review packet was emitted and matches the *current* code (tree
//!    fingerprint) - a reviewer must have seen what actually ships
//!  - review.md parses and records who reviewed; no blocker/major finding is
//!    left open, and any waived blocker/major carries a waiver rationale
//!  - the reviewer differs from the implementer, when both are known
//!  - the build/lint/test evidence is not stale: if the tree changed since
//!    the last gate passed (the review-fix loop), Gate re-runs those
//!    commands here
//!
//! Whether the review was *thorough* is judgment and lives in the REVIEW
//! playbook; the gate only checks that a distinct reviewer signed off on
//! the final code with no load-bearing findings left hanging.

use std::path::Path;

use crate::artifacts::review::{blocking_findings, parse_review_file, unjustified_waivers, Review};
use crate::core::exec::run_command;
use crate::core::git::{changed_files, is_gate_bookkeeping, tree_fingerprint};
use crate::core::paths::run_paths;
use crate::core::run::HistoryEvent;
use crate::core::state_machine::Phase;
use crate::core::targets::{check_name, resolve_phase_targets};
use crate::core::trust::is_commands_trusted;
use crate::gates::plan_drift::plan_drift_check;
use crate::gates::types::{
    fail, fail_untrusted, pass, pass_advisory, result, Check, GateContext, GateResult,
};

pub fn review_gate(ctx: &GateContext) -> GateResult {
    let mut checks: Vec<Check> = Vec::new();
    if let Some(drift) = plan_drift_check(ctx, "review.plan-drift") {
        checks.push(drift);
    }
    let paths = run_paths(&ctx.root, &ctx.run.id);
    let current = tree_fingerprint(&ctx.root);

    checks.push(packet_check(ctx, &paths.review_packet, current.as_deref()));

    let parsed = parse_review_file(&paths.review);
    match parsed.review {
        None => checks.push(fail(
            "review.findings",
            format!("review.md invalid: {}", parsed.errors.join("; ")),
        )),
        Some(review) => {
            checks.push(findings_check(&review));
            checks.push(reviewer_check(ctx, &review));
        }
    }

    let touched: Vec<String> = changed_files(&ctx.root, ctx.run.base_ref.as_deref())
        .into_iter()
        .filter(|f| !is_gate_bookkeeping(&ctx.root, f))
        .collect();
    checks.extend(evidence_checks(ctx, current.as_deref(), &touched));

    result(Phase::Review, checks)
}

/// The packet must exist and have been generated from the code as it stands
/// now; otherwise the reviewer signed off on a different diff. Without git
/// the fingerprint is unavailable - existence is all we can check, and we
/// say so.
fn packet_check(ctx: &GateContext, packet_path: &Path, current: Option<&str>) -> Check {
    if ctx.run.review.is_none() || !packet_path.exists() {
        return fail(
            "review.packet",
            "no review packet - run `gate review --fresh` to emit one",
        );
    }
    let Some(current) = current else {
        return pass(
            "review.packet",
            "packet emitted (not a git repo - freshness unverifiable)",
        );
    };
    let review = ctx.run.review.as_ref().unwrap();
    if review.tree_hash.as_deref() != Some(current) {
        return fail(
            "review.packet",
            "packet is stale - the code changed after it was generated; re-run `gate review --fresh` \
so the reviewer sees the code that ships",
        );
    }
    pass("review.packet", "review packet matches the current code")
}

fn findings_check(review: &Review) -> Check {
    let blocking = blocking_findings(review);
    if !blocking.is_empty() {
        let list = blocking
            .iter()
            .map(|f| format!("{} ({})", f.id, f.severity.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        return fail(
            "review.findings",
            format!("unresolved blocker/major findings: {list}"),
        );
    }
    let no_waiver = unjustified_waivers(review);
    if !no_waiver.is_empty() {
        let list = no_waiver
            .iter()
            .map(|f| f.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return fail(
            "review.findings",
            format!("waived findings missing a rationale: {list} (add a `waiver:`)"),
        );
    }
    pass(
        "review.findings",
        format!("{} finding(s); none blocking", review.findings.len()),
    )
}

/// The reviewer signs review.md (`reviewer:` in its frontmatter) - identity
/// is claimed at sign-off time, not when the packet was emitted. A missing
/// name fails: it is the cheapest mechanical proof that *someone* went
/// through the rubric, and without it `gate review --fresh && gate next`
/// would pass on the untouched scaffold. Equal to the implementer's session
/// id fails (self-review); an unknown implementer can't be compared at all,
/// so this passes *advisory* rather than verified - a CLI cannot prove a
/// human, and folding "we couldn't check" into a plain checkmark would
/// overstate what Gate actually confirmed.
fn reviewer_check(ctx: &GateContext, review: &Review) -> Check {
    if review.reviewer.is_empty() {
        return fail(
            "review.reviewer",
            "review.md does not say who reviewed - the reviewer must fill in `reviewer:` when signing off",
        );
    }
    let implementer = ctx.run.session_id.as_deref();
    if let Some(implementer) = implementer {
        if review.reviewer == implementer {
            return fail(
                "review.reviewer",
                format!(
                    "reviewer ({}) is the implementer - get a fresh pair of eyes",
                    review.reviewer
                ),
            );
        }
    }
    match implementer {
        Some(_) => pass(
            "review.reviewer",
            format!("reviewed by {} (\u{2260} implementer)", review.reviewer),
        ),
        None => pass_advisory(
            "review.reviewer",
            format!(
                "reviewed by {} - independence unverified: no implementer session id was \
recorded for this run, so Gate could not compare it against the reviewer (advisory only, not blocking)",
                review.reviewer
            ),
        ),
    }
}

/// Staleness guard for the review-fix loop: fixing a finding changes the
/// code *after* IMPLEMENT/TEST certified it, and DONE is one `gate next`
/// away. When the current tree still matches the fingerprint recorded at
/// the last gate pass, the earlier evidence stands; otherwise Gate re-runs
/// build/lint/test right here and requires them green. Fails closed -
/// untrusted commands are never spawned, and a red re-run blocks DONE.
///
/// Targets (Milestone 3): resolved via `resolve_phase_targets`, mirroring
/// the IMPLEMENT/TEST gates. Reading only `ctx.config.commands` (the
/// top-level set) let a per-target-only repo - no top-level build/lint/test
/// configured at all - pass this check vacuously, never re-verifying any
/// target's actual commands. Bare/legacy case (no targets configured, or
/// none affected) collapses to a single unbracketed "review.evidence"
/// check, byte-identical to before targets existed.
fn evidence_checks(ctx: &GateContext, current: Option<&str>, touched: &[String]) -> Vec<Check> {
    let last_verified = ctx
        .run
        .history
        .iter()
        .rev()
        .find(|h| h.event == HistoryEvent::Passed && h.tree_hash.is_some())
        .and_then(|h| h.tree_hash.as_deref());
    if let (Some(current), Some(last)) = (current, last_verified) {
        if current == last {
            return vec![pass(
                "review.evidence",
                "code unchanged since the last gate passed - evidence still valid",
            )];
        }
    }

    let trusted = is_commands_trusted(&ctx.root);
    resolve_phase_targets(&ctx.config, ctx.run.target_override.as_deref(), touched)
        .into_iter()
        .map(|t| {
            let name = check_name("review.evidence", t.target.as_deref());
            let to_run: Vec<(&'static str, &str)> = [
                ("build", t.commands.build.as_deref()),
                ("lint", t.commands.lint.as_deref()),
                ("test", t.commands.test.as_deref()),
            ]
            .into_iter()
            .filter_map(|(label, cmd)| cmd.map(|c| (label, c)))
            .collect();
            if to_run.is_empty() {
                return pass(name, "no build/lint/test commands configured - nothing to re-verify");
            }
            if !trusted {
                return fail_untrusted(
                    name,
                    "code changed since the last gate passed and commands are untrusted - run `gate trust`",
                );
            }
            let mut red: Vec<String> = Vec::new();
            for &(label, cmd) in &to_run {
                let res = run_command(cmd, ctx.root.to_str().unwrap_or("."), None);
                if res.code != 0 {
                    red.push(format!("{label} (exit {})", res.code));
                }
            }
            if red.is_empty() {
                let labels = to_run.iter().map(|(l, _)| *l).collect::<Vec<_>>().join("/");
                pass(name, format!("code changed since the last gate passed - re-verified: {labels} green"))
            } else {
                fail(
                    name,
                    format!("code changed since the last gate passed and re-verification failed: {}", red.join(", ")),
                )
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::GateConfig;
    use crate::core::run::{new_run, now_iso, HistoryEntry, NewRunParams, ReviewRequest};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command as StdCommand;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-review-gate-rs-{name}-{}", std::process::id()));
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

    fn write_review(root: &Path, content: &str) {
        write_file(root, ".gate/runs/r1/review.md", content);
    }
    fn write_packet(root: &Path) {
        write_file(root, ".gate/runs/r1/review-packet.md", "# packet");
    }

    const SIGNED_EMPTY_REVIEW: &str = "---\nreviewer: rev-1\nfindings: []\n---\n# Review";

    fn review_run(root: &Path, session_id: Option<&str>) -> crate::core::run::Run {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: session_id.map(String::from),
            target_override: None,
        });
        run.phase = Phase::Review;
        run.review = Some(ReviewRequest {
            requested_by: None,
            requested_at: now_iso(),
            tree_hash: tree_fingerprint(root),
        });
        run
    }

    fn check<'a>(res: &'a GateResult, name: &str) -> Option<&'a Check> {
        res.checks.iter().find(|c| c.name == name)
    }

    #[test]
    fn passes_with_a_fresh_packet_a_signed_review_and_no_blocking_findings() {
        let root = make_repo("review-pass");
        let run = review_run(&root, None);
        write_packet(&root);
        write_review(&root, SIGNED_EMPTY_REVIEW);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        assert!(res.ok, "{:?}", res.checks);
        assert!(check(&res, "review.packet").unwrap().ok);
        assert!(check(&res, "review.findings").unwrap().ok);
        assert!(check(&res, "review.reviewer").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_no_packet_was_emitted() {
        let root = make_repo("review-no-packet");
        write_review(&root, SIGNED_EMPTY_REVIEW);
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.phase = Phase::Review; // no run.review set
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        assert!(!check(&res, "review.packet").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_the_packet_is_stale() {
        let root = make_repo("review-stale-packet");
        let run = review_run(&root, None);
        write_packet(&root);
        write_review(&root, SIGNED_EMPTY_REVIEW);
        write_file(&root, "sneaky.js", "changed after the reviewer looked\n");
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        assert!(!check(&res, "review.packet").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_an_unsigned_review() {
        let root = make_repo("review-unsigned");
        let run = review_run(&root, None);
        write_packet(&root);
        write_review(&root, "---\nreviewer:\nfindings: []\n---\n# Review");
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        assert!(!check(&res, "review.reviewer").unwrap().ok);
        assert!(!res.ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn blocks_on_an_open_blocker_finding() {
        let root = make_repo("review-open-blocker");
        let run = review_run(&root, None);
        write_packet(&root);
        write_review(
            &root,
            "---\nreviewer: rev-1\nfindings:\n  - id: f1\n    severity: blocker\n    status: open\n    note: bad\n---\n",
        );
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        assert!(!check(&res, "review.findings").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn accepts_a_waived_major_finding_with_a_rationale() {
        let root = make_repo("review-waived-ok");
        let run = review_run(&root, None);
        write_packet(&root);
        write_review(
            &root,
            "---\nreviewer: rev-1\nfindings:\n  - id: f1\n    severity: major\n    status: waived\n    note: n\n    waiver: \"accepted for now\"\n---\n",
        );
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        assert!(check(&res, "review.findings").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_a_waived_major_finding_with_no_rationale() {
        let root = make_repo("review-waived-no-rationale");
        let run = review_run(&root, None);
        write_packet(&root);
        write_review(
            &root,
            "---\nreviewer: rev-1\nfindings:\n  - id: f1\n    severity: major\n    status: waived\n    note: n\n---\n",
        );
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        assert!(!check(&res, "review.findings").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_self_review_when_the_signing_reviewer_equals_the_implementer() {
        let root = make_repo("review-self-review");
        let run = review_run(&root, Some("agent-1"));
        write_packet(&root);
        write_review(&root, "---\nreviewer: agent-1\nfindings: []\n---\n");
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        assert!(!check(&res, "review.reviewer").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn passes_when_the_signing_reviewer_differs_from_the_implementer() {
        let root = make_repo("review-distinct-reviewer");
        let run = review_run(&root, Some("agent-1"));
        write_packet(&root);
        write_review(&root, "---\nreviewer: agent-2\nfindings: []\n---\n");
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        let c = check(&res, "review.reviewer").unwrap();
        assert!(c.ok);
        assert!(
            !c.advisory,
            "a verified distinct reviewer must not read as advisory"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reviewer_independence_is_advisory_not_verified_when_no_implementer_session_id_is_known() {
        let root = make_repo("review-reviewer-unknown-implementer");
        let run = review_run(&root, None);
        write_packet(&root);
        write_review(&root, SIGNED_EMPTY_REVIEW);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = review_gate(&ctx);
        let c = check(&res, "review.reviewer").unwrap();
        assert!(c.ok, "must still be non-blocking");
        assert!(
            c.advisory,
            "unverifiable independence must render distinctly from a verified pass"
        );
        assert!(c.detail.contains("independence unverified"));
        fs::remove_dir_all(&root).unwrap();
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

    #[test]
    fn skips_re_verification_while_the_tree_matches_the_last_passed_gate() {
        let root = make_repo("review-evidence-skip");
        let mut run = review_run(&root, None);
        // TEST passed on exactly this tree; the sentinel command must not run.
        run.history.push(HistoryEntry {
            phase: Phase::Test,
            event: HistoryEvent::Passed,
            at: now_iso(),
            detail: None,
            tree_hash: tree_fingerprint(&root),
        });
        let config = write_config(&root, "commands:\n  test: \"touch ran.txt\"\n", true);
        write_packet(&root);
        write_review(&root, SIGNED_EMPTY_REVIEW);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config,
        };
        let res = review_gate(&ctx);
        assert!(check(&res, "review.evidence").unwrap().ok);
        assert!(!root.join("ran.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn re_verifies_and_fails_when_a_review_fix_left_the_suite_red() {
        let root = make_repo("review-evidence-red");
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.phase = Phase::Review;
        run.history.push(HistoryEntry {
            phase: Phase::Test,
            event: HistoryEvent::Passed,
            at: now_iso(),
            detail: None,
            tree_hash: Some("sha256:before-the-fix".to_string()),
        });
        let cmd = "node -e \"process.exit(1)\"";
        let config = write_config(
            &root,
            &format!("commands:\n  test: {}\n", yaml_quote(cmd)),
            true,
        );
        write_file(&root, "fix.js", "the review fix\n");
        run.review = Some(ReviewRequest {
            requested_by: None,
            requested_at: now_iso(),
            tree_hash: tree_fingerprint(&root),
        });
        write_packet(&root);
        write_review(&root, SIGNED_EMPTY_REVIEW);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config,
        };
        let res = review_gate(&ctx);
        assert!(!check(&res, "review.evidence").unwrap().ok);
        assert!(!res.ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn re_verifies_and_passes_when_the_review_fix_keeps_everything_green() {
        let root = make_repo("review-evidence-green");
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.phase = Phase::Review;
        run.history.push(HistoryEntry {
            phase: Phase::Test,
            event: HistoryEvent::Passed,
            at: now_iso(),
            detail: None,
            tree_hash: Some("sha256:before-the-fix".to_string()),
        });
        let cmd = "node -e \"process.exit(0)\"";
        let config = write_config(
            &root,
            &format!("commands:\n  test: {}\n", yaml_quote(cmd)),
            true,
        );
        write_file(&root, "fix.js", "the review fix\n");
        run.review = Some(ReviewRequest {
            requested_by: None,
            requested_at: now_iso(),
            tree_hash: tree_fingerprint(&root),
        });
        write_packet(&root);
        write_review(&root, SIGNED_EMPTY_REVIEW);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config,
        };
        let res = review_gate(&ctx);
        assert!(check(&res, "review.evidence").unwrap().ok);
        assert!(res.ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_re_verify_with_untrusted_commands() {
        let root = make_repo("review-evidence-untrusted");
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.phase = Phase::Review;
        run.history.push(HistoryEntry {
            phase: Phase::Test,
            event: HistoryEvent::Passed,
            at: now_iso(),
            detail: None,
            tree_hash: Some("sha256:before-the-fix".to_string()),
        });
        let config = write_config(&root, "commands:\n  test: \"touch ran.txt\"\n", false);
        run.review = Some(ReviewRequest {
            requested_by: None,
            requested_at: now_iso(),
            tree_hash: tree_fingerprint(&root),
        });
        write_packet(&root);
        write_review(&root, SIGNED_EMPTY_REVIEW);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config,
        };
        let res = review_gate(&ctx);
        assert!(!check(&res, "review.evidence").unwrap().ok);
        assert!(!root.join("ran.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn actually_re_runs_a_per_target_only_repos_commands_instead_of_passing_vacuously() {
        let root = make_repo("review-evidence-target-only");
        // No top-level build/lint/test at all - only the api target has commands.
        let config = write_config(
            &root,
            "targets:\n  api:\n    match:\n      - apps/api/**\n    commands:\n      test: \"node -e \\\"process.exit(1)\\\"\"\n",
            true,
        );
        write_file(&root, "apps/api/x.py", "changed\n");
        let mut run = review_run(&root, None);
        run.history.push(HistoryEntry {
            phase: Phase::Test,
            event: HistoryEvent::Passed,
            at: now_iso(),
            detail: None,
            tree_hash: Some("sha256:stale".to_string()),
        });
        write_review(&root, SIGNED_EMPTY_REVIEW);
        write_packet(&root);
        let ctx = GateContext {
            root: root.clone(),
            run,
            config,
        };
        let res = review_gate(&ctx);
        assert!(check(&res, "review.evidence[api]").is_some());
        assert!(check(&res, "review.evidence").is_none());
        assert!(!check(&res, "review.evidence[api]").unwrap().ok);
        assert!(!res.ok);
        fs::remove_dir_all(&root).unwrap();
    }
}
