//! Port of `src/commands/advance.ts` (`advance`: moves a run to its next
//! phase and records the transition). Ported alongside the commands that
//! call it (wave 4, see `docs/rust-port.md`): `next.rs` (this wave) and
//! `skip.rs` (the other wave-4 group).

use std::path::Path;

use crate::artifacts::retro::RETRO_TEMPLATE;
use crate::cli::output::UserError;
use crate::core::current::clear_run_everywhere;
use crate::core::fsx::write_file_atomic;
use crate::core::paths::run_paths;
use crate::core::run::{now_iso, write_run, HistoryEntry, HistoryEvent, Run, RunStatus};
use crate::core::state_machine::{is_terminal, next_phase, Phase};

/// The phase transition `advance` just made. Adaptation note: TS's
/// `advance` throws on a `writeRun`/`clearRunEverywhere` failure (an
/// uncaught exception propagating to `cli.ts`'s top-level handler); this
/// port returns `Result` instead, the same idiom already used throughout
/// `core::current`/`core::run` for I/O that TS lets throw.
pub struct Advance {
    pub from: Phase,
    pub to: Phase,
}

/// Advance a run out of its current phase and persist it. Shared by `gate
/// next` (on a passing gate) and `gate skip` (on a human override) so the
/// terminal / current-run bookkeeping lives in one place.
pub fn advance(root: &Path, run: &mut Run) -> Result<Advance, UserError> {
    let from = run.phase;
    if let Some(to) = next_phase(from, &run.profile) {
        run.phase = to;
        run.history.push(HistoryEntry {
            phase: to,
            event: HistoryEvent::Entered,
            at: now_iso(),
            detail: None,
            tree_hash: None,
        });
        if is_terminal(to) {
            run.status = RunStatus::Done;
        }
        if to == Phase::Retro {
            scaffold_retro_if_missing(root, run);
        }
    }
    write_run(root, run).map_err(|e| UserError::new(e.to_string()))?;
    // Cleared by scanning current.json for whichever key maps to this run
    // id, not by trusting run.branch: a run's own branch record can be
    // stale or never backfilled, and clearing by the wrong key would leave
    // the real mapping - and this now-finished run - resolvable forever.
    if is_terminal(run.phase) {
        clear_run_everywhere(root, &run.id)?;
    }
    Ok(Advance {
        from,
        to: run.phase,
    })
}

/// `gate start` scaffolds retro.md up front for any run whose profile walks
/// through RETRO - but a run started before that existed (pre-Milestone-3)
/// would enter RETRO with no retro.md and no way to satisfy the gate.
/// Repair it here, on the transition itself, so no run can stall
/// permanently.
fn scaffold_retro_if_missing(root: &Path, run: &Run) {
    let retro = run_paths(root, &run.id).retro;
    if !retro.exists() {
        let _ = write_file_atomic(&retro, &RETRO_TEMPLATE.replace("%TITLE%", &run.title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::run::{new_run, NewRunParams};
    use std::fs;

    fn tmp_repo(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-advance-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_with_phase(phase: Phase) -> Run {
        let mut run = new_run(NewRunParams {
            id: "r1".to_string(),
            title: "pre-upgrade run".to_string(),
            profile: "feature".to_string(),
            branch: None,
            base_ref: None,
            session_id: None,
            target_override: None,
        });
        run.phase = phase;
        run
    }

    #[test]
    fn writes_retro_md_for_a_run_started_before_retro_scaffolding_existed() {
        let root = tmp_repo("scaffold");
        let mut run = run_with_phase(Phase::Review); // about to advance into RETRO
        let retro_path = run_paths(&root, "r1").retro;
        assert!(!retro_path.exists());

        let result = advance(&root, &mut run).unwrap();

        assert_eq!(result.to, Phase::Retro);
        assert!(retro_path.exists());
        let content = fs::read_to_string(&retro_path).unwrap();
        assert!(content.contains("broke: []"));
        assert!(content.contains("pre-upgrade run"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_clobber_an_existing_retro_md() {
        let root = tmp_repo("no-clobber");
        let mut run = run_with_phase(Phase::Review);
        let retro_path = run_paths(&root, "r1").retro;
        let custom =
            "---\nbroke:\n  - already filled in\navoid: []\nconventions: []\n---\n# Retro\n";
        fs::create_dir_all(run_paths(&root, "r1").dir).unwrap();
        fs::write(&retro_path, custom).unwrap();

        advance(&root, &mut run).unwrap();

        assert_eq!(fs::read_to_string(&retro_path).unwrap(), custom);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn does_not_scaffold_retro_md_when_the_transition_is_not_into_retro() {
        let root = tmp_repo("not-retro");
        let mut run = run_with_phase(Phase::Plan); // -> IMPLEMENT, not RETRO
        advance(&root, &mut run).unwrap();
        assert!(!run_paths(&root, "r1").retro.exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
