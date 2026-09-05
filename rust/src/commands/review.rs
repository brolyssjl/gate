//! Port of `src/commands/review.ts`.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

use crate::artifacts::review::{
    parse_review_file, serialize_review, Finding, FindingStatus, Severity,
};
use crate::cli::args::{parse_args, ParsedArgs};
use crate::cli::context::{require_active_run, ActiveContext};
use crate::cli::output::{emit, UserError};
use crate::core::config::GateConfig;
use crate::core::fsx::write_file_atomic;
use crate::core::git::{diff_text, tree_fingerprint};
use crate::core::identity;
use crate::core::json::Value;
use crate::core::paths::run_paths;
use crate::core::playbooks::resolve_playbook_with_overlays;
use crate::core::run::{now_iso, write_run, ArtifactEntry, ReviewRequest, Run};
use crate::core::state_machine::Phase;
use crate::core::targets::resolve_display_targets;

const REVIEW_TEMPLATE: &str = r#"---
reviewer:
findings: []
# The reviewer fills in `reviewer:` when signing off - the gate refuses an
# anonymous review. Each finding: { id, severity: blocker|major|minor|nit,
#   status: open|resolved|waived, note: "what's wrong",
#   waiver: "why it's acceptable" (required when waived) }
---

# Review

Record findings in the frontmatter above. The gate blocks on any open
blocker/major finding; minor/nit are advisory. Resolve by fixing the code
(Gate re-verifies build/lint/test and needs a fresh packet after any fix), or
waive with a human rationale.
"#;

enum PacketResult {
    Existing {
        packet: String,
        findings_file: String,
        rubric: String,
    },
    Regenerated {
        packet: String,
        findings_file: String,
        rubric: String,
        requested_by: Option<String>,
        tree_hash: Option<String>,
        plan: String,
        diff: String,
        packet_text: String,
    },
}

/// `run.artifacts["name"] = entry` in TS: replace an existing key in
/// place (preserving its original position), or append a new one -
/// matching JS object-assignment ordering semantics for `Run::artifacts`'s
/// insertion-ordered `Vec` representation.
fn set_artifact(run: &mut Run, name: &str, entry: ArtifactEntry) {
    if let Some(slot) = run.artifacts.iter_mut().find(|(k, _)| k == name) {
        slot.1 = entry;
    } else {
        run.artifacts.push((name.to_string(), entry));
    }
}

/// Emit the self-contained review packet (diff + plan + rubric) and
/// scaffold review.md, unless one already exists and `--fresh` wasn't
/// passed - shared by the default (agent-facing) path and `--human`, so
/// both see exactly the same packet-freshness rules.
fn ensure_packet(
    root: &Path,
    run: &mut Run,
    config: &GateConfig,
    args: &ParsedArgs,
) -> Result<PacketResult, UserError> {
    let paths = run_paths(root, &run.id);
    let fresh = args.flags.is_true("fresh");

    if !paths.review.exists() {
        write_file_atomic(&paths.review, REVIEW_TEMPLATE)
            .map_err(|e| UserError::new(e.to_string()))?;
    }

    let target_names = resolve_display_targets(
        root,
        &run.id,
        run.base_ref.as_deref(),
        run.target_override.as_deref(),
        config,
    );
    let rubric = resolve_playbook_with_overlays(root, Phase::Review, config, &target_names)
        .unwrap_or_else(|| "(no REVIEW playbook found)".to_string());

    if paths.review_packet.exists() && !fresh {
        return Ok(PacketResult::Existing {
            packet: paths.review_packet.display().to_string(),
            findings_file: paths.review.display().to_string(),
            rubric,
        });
    }

    let plan = if paths.plan.exists() {
        fs::read_to_string(&paths.plan)
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        "(no plan.md)".to_string()
    };
    let diff_raw = diff_text(root, run.base_ref.as_deref());
    let diff_trimmed = diff_raw.trim();
    let diff = if diff_trimmed.is_empty() {
        "(no diff)".to_string()
    } else {
        diff_trimmed.to_string()
    };
    let tree_hash = tree_fingerprint(root);

    let packet = [
        format!("# Review packet - {}", run.id),
        format!("\nTitle: {}", run.title),
        format!(
            "Base: {}",
            run.base_ref
                .clone()
                .unwrap_or_else(|| "(no git base)".to_string())
        ),
        format!(
            "Tree: {}",
            tree_hash
                .clone()
                .unwrap_or_else(|| "(no git tree)".to_string())
        ),
        format!("Generated: {}", now_iso()),
        format!("\n## Plan\n\n{plan}"),
        format!("\n## Rubric\n\n{}", rubric.trim()),
        format!("\n## Diff\n\n```diff\n{diff}\n```"),
        format!("\nRecord findings in: {}", paths.review.display()),
        String::new(),
    ]
    .join("\n");
    write_file_atomic(&paths.review_packet, &packet).map_err(|e| UserError::new(e.to_string()))?;

    // Requesting a packet is not a sign-off (that happens at reviewer:
    // in review.md), so no require_if_configured here - just the #29
    // fallback chain instead of the old --by/env-only pattern (#33).
    let requested_by = identity::resolve(args, root);
    run.review = Some(ReviewRequest {
        requested_by: requested_by.clone(),
        requested_at: now_iso(),
        tree_hash: tree_hash.clone(),
    });
    set_artifact(
        run,
        "review-packet.md",
        ArtifactEntry {
            phase: run.phase,
            at: now_iso(),
        },
    );
    write_run(root, run).map_err(|e| UserError::new(e.to_string()))?;

    Ok(PacketResult::Regenerated {
        packet: paths.review_packet.display().to_string(),
        findings_file: paths.review.display().to_string(),
        rubric,
        requested_by,
        tree_hash,
        plan,
        diff,
        packet_text: packet,
    })
}

/// `gate review [--fresh] [--by]` - emit a self-contained review packet
/// (diff + plan + rubric) so a *fresh* reviewer needs no prior context, and
/// scaffold review.md for their findings. `--human` (Milestone 4) walks the
/// rubric as terminal prompts instead.
pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // See `check::run`'s comment: prepend a placeholder command token so
    // `parse_args` never misreads a leading flag/positional as the command.
    let mut full = vec!["review".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let ctx = require_active_run(Some(&args))?;
    let ActiveContext {
        root,
        mut run,
        config,
    } = ctx;

    if run.phase != Phase::Review {
        return Err(UserError::new(format!(
            "nothing to review - run is in {}, not REVIEW",
            run.phase
        )));
    }

    if args.flags.is_true("human") {
        return cmd_review_human(&root, &mut run, &config, &args);
    }

    let res = ensure_packet(&root, &mut run, &config, &args)?;

    match res {
        PacketResult::Existing {
            packet,
            findings_file,
            rubric: _,
        } => {
            let human = format!(
                "A review packet already exists: {packet}\nPass --fresh to regenerate it from the current code (required after any fix)."
            );
            let mut data = Value::object();
            data.insert("phase", run.phase.as_str());
            data.insert("packet", packet);
            data.insert("findingsFile", findings_file);
            data.insert("regenerated", false);
            emit(&human, &data, &args.flags)
        }
        PacketResult::Regenerated {
            packet,
            findings_file,
            rubric,
            requested_by,
            tree_hash,
            plan,
            diff,
            packet_text,
        } => {
            let human = [
                format!("Review packet written to: {packet}"),
                format!("Record findings in:        {findings_file}"),
                String::new(),
                "The reviewer signs off by filling in `reviewer:` in review.md. Resolve or"
                    .to_string(),
                "waive every blocker/major finding, then run `gate next`.".to_string(),
                String::new(),
                packet_text,
            ]
            .join("\n");
            let mut data = Value::object();
            data.insert("phase", run.phase.as_str());
            data.insert("packet", packet);
            data.insert("findingsFile", findings_file);
            data.insert("regenerated", true);
            data.insert("requestedBy", requested_by);
            data.insert("treeHash", tree_hash);
            data.insert("plan", plan);
            data.insert("diff", diff);
            data.insert("rubric", rubric);
            emit(&human, &data, &args.flags)
        }
    }
}

type Lines = io::Lines<io::StdinLock<'static>>;

/// A minimal line-prompter over stdin. Rust's `io::Lines` reads one
/// buffered line at a time with no equivalent of Node readline's dropped-
/// buffered-line hazard (see the TS doc comment on `createPrompter`), so a
/// synchronous read-per-prompt is sufficient here.
fn ask(lines: &mut Lines, prompt: &str) -> Result<String, UserError> {
    print!("{prompt}");
    let _ = io::stdout().flush();
    match lines.next() {
        Some(Ok(line)) => Ok(line.trim().to_string()),
        _ => Err(UserError::new(
            "gate review --human: input ended before the review was recorded",
        )),
    }
}

const SEVERITY_CHOICES: [&str; 4] = ["blocker", "major", "minor", "nit"];
const STATUS_CHOICES: [&str; 3] = ["open", "resolved", "waived"];

/// The default id is the lowest-numbered `f<N>` not already taken - not
/// just `existing.len() + 1`, which collides the moment an earlier finding
/// was given (or kept) an id out of strict f1/f2/f3 sequence.
fn next_default_id(existing: &[Finding]) -> String {
    let used: Vec<&str> = existing.iter().map(|f| f.id.as_str()).collect();
    let mut n = existing.len() + 1;
    while used.contains(&format!("f{n}").as_str()) {
        n += 1;
    }
    format!("f{n}")
}

fn ask_choice(lines: &mut Lines, label: &str, choices: &[&str]) -> Result<String, UserError> {
    loop {
        let ans = ask(lines, &format!("{label} ({}): ", choices.join("/")))?.to_lowercase();
        if choices.contains(&ans.as_str()) {
            return Ok(ans);
        }
        println!("    invalid - choose one of {}", choices.join(", "));
    }
}

fn ask_finding(lines: &mut Lines, existing: &[Finding]) -> Result<Finding, UserError> {
    let used: Vec<String> = existing.iter().map(|f| f.id.clone()).collect();
    let default_id = next_default_id(existing);
    let answer = ask(lines, &format!("  id [{default_id}]: "))?;
    let mut id = if answer.is_empty() {
        default_id.clone()
    } else {
        answer
    };
    while used.contains(&id) {
        let answer = ask(
            lines,
            &format!("  id \"{id}\" is already used - choose another [{default_id}]: "),
        )?;
        id = if answer.is_empty() {
            default_id.clone()
        } else {
            answer
        };
    }
    let severity_str = ask_choice(lines, "  severity", &SEVERITY_CHOICES)?;
    let severity =
        Severity::from_str_opt(&severity_str).expect("ask_choice only returns a valid severity");
    let note = ask(lines, "  note (what's wrong): ")?;
    let status_str = ask_choice(lines, "  status", &STATUS_CHOICES)?;
    let status =
        FindingStatus::from_str_opt(&status_str).expect("ask_choice only returns a valid status");
    let mut waiver = String::new();
    if status == FindingStatus::Waived {
        while waiver.is_empty() {
            waiver = ask(lines, "  waiver rationale (required): ")?;
        }
    }
    Ok(Finding {
        id,
        severity,
        status,
        note,
        waiver,
    })
}

fn finding_to_json(f: &Finding) -> Value {
    let mut v = Value::object();
    v.insert("id", f.id.as_str());
    v.insert("severity", f.severity.as_str());
    v.insert("status", f.status.as_str());
    v.insert("note", f.note.as_str());
    v.insert("waiver", f.waiver.as_str());
    v
}

/// `gate review --human` (Milestone 4): a minimal terminal rubric walk for
/// solo devs with no second agent/human session to hand the packet to.
/// Emits the same packet the agent-facing path does, then prompts for
/// findings and a reviewer identity, and writes review.md in the exact
/// shape the REVIEW gate parses.
fn cmd_review_human(
    root: &Path,
    run: &mut Run,
    config: &GateConfig,
    args: &ParsedArgs,
) -> Result<(), UserError> {
    let res = ensure_packet(root, run, config, args)?;
    let (packet, findings_file, rubric, regenerated) = match res {
        PacketResult::Existing {
            packet,
            findings_file,
            rubric,
        } => (packet, findings_file, rubric, false),
        PacketResult::Regenerated {
            packet,
            findings_file,
            rubric,
            ..
        } => (packet, findings_file, rubric, true),
    };

    print!("\nReview packet: {packet}\n");
    print!(
        "{}",
        if regenerated {
            "(freshly generated)\n"
        } else {
            "(reusing an existing packet - pass --fresh to regenerate)\n"
        }
    );
    print!("\n{}\n\n", rubric.trim());
    print!(
        "Read the diff in the packet above, then walk the rubric. Record each finding when prompted;\nminor/nit are advisory, blocker/major must be resolved or waived with a rationale.\n\n"
    );
    let _ = io::stdout().flush();

    // A parse failure on a *pre-existing, non-scaffold* review.md must not
    // silently discard whatever findings it already held - refuse instead
    // of guessing; the scaffold template itself always parses cleanly.
    let parsed = parse_review_file(Path::new(&findings_file));
    let Some(existing) = parsed.review else {
        return Err(UserError::new(format!(
            "cannot walk the review - existing review.md is invalid: {}",
            parsed.errors.join("; ")
        )));
    };
    let mut findings: Vec<Finding> = existing.findings;

    let mut lines: Lines = io::stdin().lock().lines();

    loop {
        let again = ask(
            &mut lines,
            &format!("Add a finding? [y/N] ({} recorded so far) ", findings.len()),
        )?
        .to_lowercase();
        if again != "y" && again != "yes" {
            break;
        }
        let finding = ask_finding(&mut lines, &findings)?;
        findings.push(finding);
    }

    let mut reviewer = args.flags.str("by").unwrap_or("").trim().to_string();
    while reviewer.is_empty() {
        reviewer = ask(&mut lines, "Reviewer name (required to sign off): ")?;
    }

    write_file_atomic(
        Path::new(&findings_file),
        &serialize_review(&reviewer, &findings),
    )
    .map_err(|e| UserError::new(e.to_string()))?;

    let blocking: Vec<&Finding> = findings
        .iter()
        .filter(|f| {
            matches!(f.severity, Severity::Blocker | Severity::Major)
                && f.status == FindingStatus::Open
        })
        .collect();
    print!(
        "\nRecorded review.md: reviewer={reviewer}, {} finding(s){}",
        findings.len(),
        if !blocking.is_empty() {
            format!(", {} still blocking\n", blocking.len())
        } else {
            ", none blocking\n".to_string()
        }
    );
    print!(
        "{}",
        if !blocking.is_empty() {
            "Resolve or waive the blocking finding(s), then `gate next`.\n"
        } else {
            "Run `gate next` to advance.\n"
        }
    );
    let _ = io::stdout().flush();

    let mut data = Value::object();
    data.insert("phase", run.phase.as_str());
    data.insert("packet", packet);
    data.insert("findingsFile", findings_file);
    data.insert("reviewer", reviewer);
    data.insert(
        "findings",
        Value::Array(findings.iter().map(finding_to_json).collect()),
    );
    emit("", &data, &args.flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_default_id_skips_a_gap_left_by_a_hand_deleted_finding() {
        let existing = vec![Finding {
            id: "f2".to_string(),
            severity: Severity::Minor,
            status: FindingStatus::Resolved,
            note: "n".to_string(),
            waiver: String::new(),
        }];
        // Old bug: existing.len() + 1 = "f2" again (length 1), colliding.
        assert_eq!(next_default_id(&existing), "f3");
    }

    #[test]
    fn next_default_id_is_f1_when_empty() {
        assert_eq!(next_default_id(&[]), "f1");
    }

    #[test]
    fn set_artifact_replaces_in_place_and_appends_new_keys() {
        let mut run = crate::core::run::new_run(crate::core::run::NewRunParams {
            id: "r1".to_string(),
            title: "t".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        set_artifact(
            &mut run,
            "a.md",
            ArtifactEntry {
                phase: Phase::Plan,
                at: "t1".to_string(),
            },
        );
        set_artifact(
            &mut run,
            "b.md",
            ArtifactEntry {
                phase: Phase::Implement,
                at: "t2".to_string(),
            },
        );
        set_artifact(
            &mut run,
            "a.md",
            ArtifactEntry {
                phase: Phase::Test,
                at: "t3".to_string(),
            },
        );
        assert_eq!(run.artifacts.len(), 2);
        assert_eq!(run.artifacts[0].0, "a.md");
        assert_eq!(run.artifacts[0].1.at, "t3");
        assert_eq!(run.artifacts[1].0, "b.md");
    }

    #[test]
    fn review_template_carries_the_scaffold_shape() {
        assert!(REVIEW_TEMPLATE.starts_with("---\nreviewer:\nfindings: []\n"));
        assert!(REVIEW_TEMPLATE.contains("# Review"));
    }
}
