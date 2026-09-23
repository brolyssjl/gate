//! Port of `src/gates/test.ts` (`testGate`, `loadReport`, `statReport`,
//! `targetReportPath`). Ported in wave 3 (see `docs/rust-port.md`),
//! alongside `test_report.rs`.
//!
//! TEST gate - deterministic:
//!  - `build` and `lint` exit 0 (parity with IMPLEMENT; the bugfix profile
//!    has no IMPLEMENT phase, so this is where its build/lint discipline
//!    lives)
//!  - the test command exits 0 (no green claim over a red suite)
//!  - every `test:`-verified acceptance criterion maps to a named passing test
//!  - no skipped tests
//!  - diff coverage >= threshold (only when coverage command + threshold are set)
//!
//! Machine evidence is produced by Gate itself: the report comes from the
//! test command Gate just ran (stdout, or a file that command wrote), never
//! from a file an agent staged in advance.
//!
//! Targets (Milestone 3): when `config.targets` resolves to a single bare,
//! top-level command set (no targets configured, or none affected by the
//! touched files), this delegates to `run_bare_target_gate` - the exact
//! same check sequence and names Gate produced before targets existed.
//! Once >=1 named target is affected, each gets its own `test.build[name]`
//! / `test.lint[name]` / `test.command[name]` / `test.coverage[name]`
//! checks; `test.criteria` / `test.no-skips` stay single, aggregate checks
//! over every affected target's combined report (a criterion is a property
//! of the plan, not of one stack).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::artifacts::plan::Criterion;
use crate::core::config::CoverageFormat;
use crate::core::exec::run_command;
use crate::core::fsx::write_file_atomic;
use crate::core::git::{changed_files, changed_lines, is_gate_bookkeeping};
use crate::core::json;
use crate::core::paths::run_paths;
use crate::core::state_machine::Phase;
use crate::core::targets::{check_name, files_for_target, resolve_phase_targets, ResolvedTarget};
use crate::core::trust::is_commands_trusted;
use crate::gates::coverage::{coverage_report_paths, diff_coverage, load_coverage};
use crate::gates::implement::command_check;
use crate::gates::plan_drift::plan_drift_check;
use crate::gates::test_report::{parse_test_report, NormalizedReport, NormalizedTest};
use crate::gates::types::{fail, fail_untrusted, pass, result, Check, GateContext, GateResult};

/// Shared by both the bare and targeted `test.criteria` failure paths: a
/// criterion verified via `verify: "test: <name>"` needs a parseable JSON
/// test report to map against, and a suite with no JSON reporter wired up
/// can never produce one - no amount of retrying fixes this on its own, so
/// the failure names the fix inline rather than just the symptom.
const NO_PARSEABLE_REPORT_FOR_CRITERIA: &str =
    "cannot verify criterion\u{2192}test mapping: no parseable test report found - \
verify: \"test: <name>\" requires the test command to emit a JSON report (on stdout, \
or written to $GATE_TEST_REPORT); if no JSON reporter is wired up, use verify: manual instead";

pub fn test_gate(ctx: &GateContext) -> GateResult {
    let report_path = run_paths(&ctx.root, &ctx.run.id).test_report;
    let touched: Vec<String> = changed_files(&ctx.root, ctx.run.base_ref.as_deref())
        .into_iter()
        .filter(|f| !is_gate_bookkeeping(&ctx.root, f))
        .collect();

    let mut checks: Vec<Check> = Vec::new();
    if let Some(drift) = plan_drift_check(ctx, "test.plan-drift") {
        checks.push(drift);
    }

    // Untrusted config never spawns a process, for any target (proposal §9)
    // - the trust hash already covers the targets block.
    if !is_commands_trusted(&ctx.root) {
        checks.push(fail_untrusted(
            "test.command",
            "test command not trusted - trust is per machine, per checkout; review .gate/config.yml and run `gate trust`",
        ));
        return result(Phase::Test, checks);
    }

    let resolved = resolve_phase_targets(&ctx.config, ctx.run.target_override.as_deref(), &touched);
    if resolved.len() == 1 && resolved[0].target.is_none() {
        checks.extend(run_bare_target_gate(ctx, &resolved[0], &report_path));
    } else {
        checks.extend(run_targeted_gate(ctx, &resolved, &report_path, &touched));
    }
    result(Phase::Test, checks)
}

fn test_env(dir: &Path, report_path: &Path) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "GATE_RUN_DIR".to_string(),
        dir.to_string_lossy().into_owned(),
    );
    env.insert(
        "GATE_TEST_REPORT".to_string(),
        report_path.to_string_lossy().into_owned(),
    );
    env
}

fn test_criteria_of(root: &Path, run_id: &str) -> Vec<Criterion> {
    crate::artifacts::plan::parse_plan_file(&run_paths(root, run_id).plan)
        .plan
        .map(|p| {
            p.criteria
                .into_iter()
                .filter(|c| c.verify.to_lowercase().starts_with("test:"))
                .collect()
        })
        .unwrap_or_default()
}

/// Byte-identical to Gate's pre-targets TEST gate: no `targets:` configured,
/// or the touched files matched none. `bare.commands`/`bare.thresholds` are
/// always exactly `ctx.config.commands`/`ctx.config.thresholds` in this
/// case.
fn run_bare_target_gate(
    ctx: &GateContext,
    bare: &ResolvedTarget,
    report_path: &Path,
) -> Vec<Check> {
    let mut checks: Vec<Check> = Vec::new();
    let dir = run_paths(&ctx.root, &ctx.run.id).dir;

    let Some(test_cmd) = bare.commands.test.clone() else {
        checks.push(fail(
            "test.command",
            "no test command configured (commands.test)",
        ));
        return checks;
    };

    checks.push(command_check(
        "test.build",
        bare.commands.build.as_deref(),
        &ctx.root,
        "build",
        true,
    ));
    checks.push(command_check(
        "test.lint",
        bare.commands.lint.as_deref(),
        &ctx.root,
        "lint",
        true,
    ));

    let report_before = stat_report(report_path);
    let env = test_env(&dir, report_path);
    let run = run_command(&test_cmd, ctx.root.to_str().unwrap_or("."), Some(&env));
    checks.push(if run.code == 0 {
        pass("test.command", "test suite passed (exit 0)")
    } else {
        let source = if !run.stderr.is_empty() {
            &run.stderr
        } else {
            &run.stdout
        };
        fail(
            "test.command",
            format!("test suite failed (exit {}): {}", run.code, tail(source)),
        )
    });

    let report = load_report(report_path, &run.stdout, report_before.as_ref());

    let test_criteria = test_criteria_of(&ctx.root, &ctx.run.id);
    if test_criteria.is_empty() {
        checks.push(pass("test.criteria", "no test-verified criteria to map"));
    } else if let Some(report) = &report {
        checks.push(criteria_check("test.criteria", &test_criteria, report));
    } else {
        checks.push(fail("test.criteria", NO_PARSEABLE_REPORT_FOR_CRITERIA));
    }

    if let Some(report) = &report {
        checks.push(if report.skipped == 0 {
            pass("test.no-skips", "no skipped tests")
        } else {
            fail(
                "test.no-skips",
                format!(
                    "{} skipped test(s); un-skip or split into a separate run",
                    report.skipped
                ),
            )
        });
    }

    if let Some(threshold) = bare.thresholds.diff_coverage {
        let all_changed = changed_lines(&ctx.root, ctx.run.base_ref.as_deref());
        checks.push(coverage_check(
            ctx,
            threshold,
            "test.coverage",
            bare.commands.coverage.as_deref(),
            &all_changed,
            None,
            bare.coverage_format,
        ));
    }

    checks
}

/// One or more named targets are affected - bracketed per-target checks,
/// aggregate criteria/skips.
fn run_targeted_gate(
    ctx: &GateContext,
    resolved: &[ResolvedTarget],
    report_path: &Path,
    touched: &[String],
) -> Vec<Check> {
    let mut checks: Vec<Check> = Vec::new();
    let mut reports: Vec<NormalizedReport> = Vec::new();
    let dir = run_paths(&ctx.root, &ctx.run.id).dir;
    // Computed once - every target and the residual-coverage check below
    // share the same changed-lines map instead of each re-invoking `git diff`.
    let all_changed = changed_lines(&ctx.root, ctx.run.base_ref.as_deref());

    for t in resolved {
        let cmd_name = check_name("test.command", t.target.as_deref());
        let Some(test_cmd) = t.commands.test.clone() else {
            checks.push(fail(cmd_name, "no test command configured (commands.test)"));
            continue;
        };
        checks.push(command_check(
            &check_name("test.build", t.target.as_deref()),
            t.commands.build.as_deref(),
            &ctx.root,
            "build",
            true,
        ));
        checks.push(command_check(
            &check_name("test.lint", t.target.as_deref()),
            t.commands.lint.as_deref(),
            &ctx.root,
            "lint",
            true,
        ));

        let path = target_report_path(report_path, t.target.as_deref());
        let report_before = stat_report(&path);
        let env = test_env(&dir, &path);
        let run = run_command(&test_cmd, ctx.root.to_str().unwrap_or("."), Some(&env));
        checks.push(if run.code == 0 {
            pass(cmd_name.clone(), "test suite passed (exit 0)")
        } else {
            let source = if !run.stderr.is_empty() {
                &run.stderr
            } else {
                &run.stdout
            };
            fail(
                cmd_name.clone(),
                format!("test suite failed (exit {}): {}", run.code, tail(source)),
            )
        });

        let report = load_report(&path, &run.stdout, report_before.as_ref());
        if let Some(r) = report {
            reports.push(r);
        } else if run.code == 0 {
            // Fail closed, mirroring the bare path: a green exit alone
            // proves nothing without a report naming what ran.
            checks.push(fail(
                check_name("test.report", t.target.as_deref()),
                "test suite passed (exit 0) but produced no parseable test report - \
emit a JSON report on stdout or write it to $GATE_TEST_REPORT",
            ));
        }

        if let (Some(threshold), Some(target_name)) = (t.thresholds.diff_coverage, &t.target) {
            let scope_files = files_for_target(&ctx.config, target_name, touched);
            checks.push(coverage_check(
                ctx,
                threshold,
                &check_name("test.coverage", Some(target_name.as_str())),
                t.commands.coverage.as_deref(),
                &all_changed,
                Some(&scope_files),
                t.coverage_format,
            ));
        }
    }

    checks.extend(residual_coverage_checks(
        ctx,
        resolved,
        touched,
        &all_changed,
    ));

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

    let test_criteria = test_criteria_of(&ctx.root, &ctx.run.id);
    if test_criteria.is_empty() {
        checks.push(pass("test.criteria", "no test-verified criteria to map"));
    } else if let Some(combined) = &combined {
        checks.push(criteria_check("test.criteria", &test_criteria, combined));
    } else {
        checks.push(fail("test.criteria", NO_PARSEABLE_REPORT_FOR_CRITERIA));
    }

    if let Some(combined) = &combined {
        checks.push(if combined.skipped == 0 {
            pass("test.no-skips", "no skipped tests")
        } else {
            fail(
                "test.no-skips",
                format!(
                    "{} skipped test(s); un-skip or split into a separate run",
                    combined.skipped
                ),
            )
        });
    }

    checks
}

/// Changed lines in files that fall under NO affected target's `match`
/// globs never get scoped into any per-target `test.coverage[name]` check
/// above, so without this they'd escape diff coverage entirely once >=1
/// target is affected. Scoped to the top-level coverage command/threshold -
/// the same command set a bare (untargeted) run would have used for these
/// files.
fn residual_coverage_checks(
    ctx: &GateContext,
    resolved: &[ResolvedTarget],
    touched: &[String],
    all_changed: &[(String, HashSet<i64>)],
) -> Vec<Check> {
    let Some(threshold) = ctx.config.thresholds.diff_coverage else {
        return Vec::new();
    };
    let mut matched: HashSet<String> = HashSet::new();
    for t in resolved {
        if let Some(name) = &t.target {
            for f in files_for_target(&ctx.config, name, touched) {
                matched.insert(f);
            }
        }
    }
    let unmatched: Vec<String> = touched
        .iter()
        .filter(|f| !matched.contains(*f))
        .cloned()
        .collect();
    if unmatched.is_empty() {
        return Vec::new();
    }
    vec![coverage_check(
        ctx,
        threshold,
        "test.coverage",
        ctx.config.commands.coverage.as_deref(),
        all_changed,
        Some(&unmatched),
        ctx.config.coverage_format,
    )]
}

fn criteria_check(name: &str, test_criteria: &[Criterion], report: &NormalizedReport) -> Check {
    let passing: Vec<String> = report
        .tests
        .iter()
        .filter(|t| t.status == crate::gates::test_report::TestStatus::Passed)
        .map(|t| t.name.to_lowercase())
        .collect();
    let unmapped: Vec<&Criterion> = test_criteria
        .iter()
        .filter(|c| {
            let idx = c.verify.find(':').map(|i| i + 1).unwrap_or(0);
            let needle = c.verify[idx..].trim().to_lowercase();
            !passing.iter().any(|n| n.contains(&needle))
        })
        .collect();
    if unmapped.is_empty() {
        pass(
            name,
            format!(
                "all {} test-verified criteria map to a passing test",
                test_criteria.len()
            ),
        )
    } else {
        let list = unmapped
            .iter()
            .map(|c| format!("{} ({})", c.id, c.verify))
            .collect::<Vec<_>>()
            .join(", ");
        fail(
            name,
            format!("criteria without a passing named test: {list}"),
        )
    }
}

fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.trim().split('\n').collect();
    let start = lines.len().saturating_sub(3);
    lines[start..].join(" \u{23ce} ")
}

/// Per-target report path: unchanged for the bare case, `<base>.<target>.json`
/// otherwise. Mirrors TS's `basePath.replace(/\.json$/, ...)`: a base path
/// not ending in `.json` is returned unchanged (the regex simply doesn't
/// match) - not a bug this port needs to fix, since `report_path` always
/// ends in `test-report.json` in practice.
pub fn target_report_path(base_path: &Path, target: Option<&str>) -> PathBuf {
    match target {
        Some(t) => {
            let s = base_path.to_string_lossy();
            match s.strip_suffix(".json") {
                Some(stripped) => PathBuf::from(format!("{stripped}.{t}.json")),
                None => base_path.to_path_buf(),
            }
        }
        None => base_path.to_path_buf(),
    }
}

/// Identity of a report file's on-disk state, for was-it-rewritten
/// detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportStat {
    mtime: std::time::SystemTime,
    size: u64,
}

pub fn stat_report(report_path: &Path) -> Option<ReportStat> {
    let meta = fs::metadata(report_path).ok()?;
    Some(ReportStat {
        mtime: meta.modified().ok()?,
        size: meta.len(),
    })
}

fn tests_to_json(tests: &[NormalizedTest]) -> json::Value {
    json::Value::Array(
        tests
            .iter()
            .map(|t| {
                let mut o = json::Value::object();
                o.insert("name", t.name.as_str());
                o.insert("status", t.status.as_str());
                o
            })
            .collect(),
    )
}

/// Normalized report for the test run Gate just executed. Evidence
/// integrity: if the agent could hand Gate the report, enforcement would be
/// fiction, so the sources are, in order:
///  1. the command's stdout (captured by Gate itself) - Gate then persists
///     the normalized report to `test-report.json` for the audit trail;
///  2. a `test-report.json` the command wrote *during this run* (runners can
///     target it via $GATE_TEST_REPORT), detected by the file's stat
///     changing across the run (`before` is the caller's stat from just
///     before spawning).
///
/// A pre-existing file staged before the run (e.g. via `gate log`) is
/// ignored. Returns `None` when neither source yields a parseable report.
pub fn load_report(
    report_path: &Path,
    stdout: &str,
    before: Option<&ReportStat>,
) -> Option<NormalizedReport> {
    if let Some(from_stdout) = parse_test_report(stdout) {
        let mut obj = json::Value::object();
        obj.insert("source", "gate: normalized from the test command's stdout");
        obj.insert("tests", tests_to_json(&from_stdout.tests));
        let text = json::stringify_pretty(&obj) + "\n";
        let _ = write_file_atomic(report_path, &text);
        return Some(from_stdout);
    }
    let after = stat_report(report_path);
    let written_during_run = match (&after, before) {
        (Some(_), None) => true,
        (Some(a), Some(b)) => a.mtime != b.mtime || a.size != b.size,
        (None, _) => false,
    };
    if written_during_run {
        if let Ok(text) = fs::read_to_string(report_path) {
            return parse_test_report(&text);
        }
    }
    None
}

/// Diff coverage. `scope_files == None` covers every changed line (the
/// bare/legacy case and the top-level residual check) - coverage command
/// run unscoped, identical to pre-targets behavior. `Some(scope_files)`
/// scopes to only the changed lines under those files (a target's `match`
/// globs, or the files matching no target at all), using the given
/// coverage command.
fn coverage_check(
    ctx: &GateContext,
    threshold: f64,
    name: &str,
    cov_cmd: Option<&str>,
    all_changed: &[(String, HashSet<i64>)],
    scope_files: Option<&[String]>,
    format: CoverageFormat,
) -> Check {
    if let Some(cmd) = cov_cmd {
        run_command(cmd, ctx.root.to_str().unwrap_or("."), None);
    }
    let Some(coverage) = load_coverage(&ctx.root, format) else {
        return fail(
            name,
            format!(
                "diff coverage threshold is {}% but no coverage report was found (expected one of {})",
                json::format_float(threshold),
                coverage_report_paths().join(", ")
            ),
        );
    };
    let changed: Vec<(String, HashSet<i64>)> = match scope_files {
        Some(files) => all_changed
            .iter()
            .filter(|(f, _)| files.contains(f))
            .cloned()
            .collect(),
        None => all_changed.to_vec(),
    };
    let dc = diff_coverage(&changed, &coverage);
    if (dc.percent as f64) >= threshold {
        return pass(
            name,
            format!(
                "diff coverage {}% \u{2265} {}%",
                dc.percent,
                json::format_float(threshold)
            ),
        );
    }
    let gap = dc
        .gaps
        .iter()
        .take(5)
        .map(|g| {
            format!(
                "{}:{}",
                g.file,
                g.uncovered
                    .iter()
                    .take(10)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    fail(
        name,
        format!(
            "diff coverage {}% < {}% - uncovered changed lines: {gap}",
            dc.percent,
            json::format_float(threshold)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::GateConfig;
    use crate::core::run::{new_run, NewRunParams};
    use std::process::Command as StdCommand;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = crate::core::testutil::unique_temp_dir(&format!("test-gate-rs-{name}"));
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
        run.phase = Phase::Test;
        run
    }

    /// Write a Node script fixture and return a shell command invoking it -
    /// keeps test-runner JS out of shell/YAML string literals entirely
    /// (no escaping to get wrong), unlike TS's inline `node -e "..."`.
    fn node_script_cmd(root: &Path, name: &str, script: &str) -> String {
        let path = root.join(name);
        fs::write(&path, script).unwrap();
        format!("node {}", path.display())
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

    fn check<'a>(res: &'a GateResult, name: &str) -> Option<&'a Check> {
        res.checks.iter().find(|c| c.name == name)
    }

    const REPORT_PASS: &str = r#"{"tests":[{"name":"greets by name","status":"passed"}]}"#;

    #[test]
    fn passes_a_green_suite_with_mapped_criteria() {
        let root = make_repo("green-suite");
        write_plan(&root, GOOD_PLAN);
        let cmd = node_script_cmd(
            &root,
            "test.js",
            &format!("console.log('{REPORT_PASS}');\n"),
        );
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        assert!(res.ok, "{:?}", res.checks);
        assert!(check(&res, "test.command").unwrap().ok);
        assert!(check(&res, "test.criteria").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_a_red_suite() {
        let root = make_repo("red-suite");
        write_plan(&root, GOOD_PLAN);
        let cmd = node_script_cmd(&root, "test.js", "process.exit(1);\n");
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        assert!(!check(&res, "test.command").unwrap().ok);
        assert!(!res.ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_a_criterion_maps_to_no_passing_test() {
        let root = make_repo("unmapped-criterion");
        write_plan(&root, GOOD_PLAN);
        let cmd = node_script_cmd(
            &root,
            "test.js",
            "console.log(JSON.stringify({tests:[{name:'unrelated',status:'passed'}]}));\n",
        );
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        assert!(!check(&res, "test.criteria").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_with_actionable_guidance_when_a_test_verified_criterion_has_no_parseable_report() {
        let root = make_repo("no-parseable-report-for-criteria");
        write_plan(&root, GOOD_PLAN);
        // A green suite that prints plain text, not JSON - the exact soak
        // scenario: `verify: "test: <name>"` in the plan, but the test
        // command has no JSON reporter wired up at all.
        let cmd = node_script_cmd(&root, "test.js", "console.log('ok');\n");
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        let criteria = check(&res, "test.criteria").unwrap();
        assert!(!criteria.ok);
        assert!(criteria.detail.contains("verify: manual"));
        assert!(criteria.detail.contains("$GATE_TEST_REPORT"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_on_skipped_tests() {
        let root = make_repo("skipped");
        write_plan(&root, GOOD_PLAN);
        let cmd = node_script_cmd(
            &root,
            "test.js",
            "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'},{name:'later',status:'skipped'}]}));\n",
        );
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        assert!(!check(&res, "test.no-skips").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_when_no_test_command_is_configured() {
        let root = make_repo("no-test-cmd");
        write_plan(&root, GOOD_PLAN);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config: GateConfig::default(),
        };
        let res = test_gate(&ctx);
        assert!(!check(&res, "test.command").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_run_an_untrusted_test_command() {
        let root = make_repo("untrusted-test-cmd");
        write_plan(&root, GOOD_PLAN);
        let config = write_test_command(&root, "touch ran.txt", false);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        assert!(!check(&res, "test.command").unwrap().ok);
        assert!(!root.join("ran.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn runs_build_and_lint_too() {
        let root = make_repo("build-and-lint");
        write_plan(&root, GOOD_PLAN);
        let cmd = node_script_cmd(
            &root,
            "test.js",
            "console.log(JSON.stringify({tests:[{name:'greets by name',status:'passed'}]}));\n",
        );
        let config = write_config(
            &root,
            &format!(
                "commands:\n  test: {}\n  build: \"exit 3\"\n",
                yaml_quote(&cmd)
            ),
            true,
        );
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        assert!(!check(&res, "test.build").unwrap().ok);
        assert!(!res.ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ignores_a_test_report_staged_before_the_run() {
        let root = make_repo("staged-report-ignored");
        write_plan(&root, GOOD_PLAN);
        let cmd = node_script_cmd(&root, "test.js", "process.exit(0);\n");
        let config = write_test_command(&root, &cmd, true);
        write_file(&root, ".gate/runs/r1/test-report.json", REPORT_PASS);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        assert!(check(&res, "test.command").unwrap().ok);
        assert!(!check(&res, "test.criteria").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn accepts_a_report_the_test_command_writes_to_gate_test_report() {
        let root = make_repo("writes-to-env-path");
        write_plan(&root, GOOD_PLAN);
        let script = format!(
            "require('fs').writeFileSync(process.env.GATE_TEST_REPORT, '{REPORT_PASS}');\n"
        );
        let cmd = node_script_cmd(&root, "test.js", &script);
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = test_gate(&ctx);
        assert!(check(&res, "test.criteria").unwrap().ok, "{:?}", res.checks);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn persists_the_normalized_report_it_parsed_from_stdout() {
        let root = make_repo("persists-normalized");
        write_plan(&root, GOOD_PLAN);
        let cmd = node_script_cmd(
            &root,
            "test.js",
            &format!("console.log('{REPORT_PASS}');\n"),
        );
        let config = write_test_command(&root, &cmd, true);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        test_gate(&ctx);
        let written = fs::read_to_string(root.join(".gate/runs/r1/test-report.json")).unwrap();
        let parsed = json::parse(&written).unwrap();
        assert!(parsed
            .get("source")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("gate"));
        assert_eq!(parsed.get("tests").unwrap().as_array().unwrap().len(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn target_report_path_appends_the_target_before_the_json_suffix() {
        let base = Path::new("/r/test-report.json");
        assert_eq!(
            target_report_path(base, Some("api")),
            PathBuf::from("/r/test-report.api.json")
        );
        assert_eq!(target_report_path(base, None), base.to_path_buf());
    }

    #[test]
    fn target_report_path_leaves_a_non_json_base_path_unchanged() {
        let base = Path::new("/r/test-report");
        assert_eq!(target_report_path(base, Some("api")), base.to_path_buf());
    }
}
