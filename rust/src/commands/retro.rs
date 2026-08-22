//! Port of `src/commands/retro.ts`.
//!
//! `gate retro` - sync the current run's retro.md into the Agnosgram
//! journal (source-linked to this run), when a store is present.
//! Phase-guarded to RETRO; refuses a substance-less retro. Idempotent:
//! re-running after a successful sync is a no-op as long as the journal
//! entry is still there. Advisory by design - with no `.agnosgram/` store,
//! this is a no-op and the RETRO gate does not require it.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifacts::retro::{has_substance, parse_retro_file};
use crate::cli::args::parse_args;
use crate::cli::context::require_active_run;
use crate::cli::output::{emit, UserError};
use crate::core::git::current_branch;
use crate::core::json::Value;
use crate::core::paths::run_paths;
use crate::core::run::{now_iso, write_run, RetroMethod, RetroSync};
use crate::core::state_machine::Phase;
use crate::integrations::agnosgram_write::{
    format_journal_entry, journal_contains_run_id, journal_file_path, write_journal_entry,
    JournalEntryParams, JournalWriteMethod,
};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn run(argv: Vec<String>) -> Result<(), UserError> {
    // See `check::run`'s comment: prepend a placeholder command token so
    // `parse_args` never misreads a leading flag/positional as the command.
    let mut full = vec!["retro".to_string()];
    full.extend(argv);
    let args = parse_args(&full);
    let ctx = require_active_run(Some(&args))?;
    let mut run = ctx.run;

    if run.phase != Phase::Retro {
        return Err(UserError::new(format!(
            "nothing to sync - run is in {}, not RETRO",
            run.phase
        )));
    }

    let paths = run_paths(&ctx.root, &run.id);
    let parsed = parse_retro_file(&paths.retro);
    let Some(retro) = parsed.retro else {
        return Err(UserError::new(format!(
            "cannot sync an invalid retro: {}",
            parsed.errors.join("; ")
        )));
    };
    if !has_substance(&retro) {
        return Err(UserError::new(
            "retro.md has no substance - answer at least one of broke/avoid/conventions before syncing",
        ));
    }

    let agnosgram_off = ctx
        .config
        .integrations
        .iter()
        .any(|(k, v)| k == "agnosgram" && v == "off");
    if agnosgram_off || !ctx.root.join(".agnosgram").exists() {
        let mut data = Value::object();
        data.insert("synced", false);
        data.insert("reason", "no-store");
        return emit(
            "No .agnosgram/ store detected - nothing to sync. The RETRO gate does not require a journal entry.",
            &data,
            &args.flags,
        );
    }

    // Single timestamp for both the entry's heading and the month file it
    // lands in - computing `when` twice could straddle a month boundary
    // and disagree on which file it actually wrote to.
    let when = now_secs();

    // Robust idempotence check: trust the recorded journalFile from a
    // prior sync, but also check the file *this* call would target right
    // now. A stale/mismatched record must not cause a duplicate entry.
    let candidate_file = journal_file_path(when);
    let already_synced = match &run.retro {
        Some(r) => {
            journal_contains_run_id(&ctx.root, &r.journal_file, &run.id)
                || journal_contains_run_id(&ctx.root, &candidate_file, &run.id)
        }
        None => false,
    };
    if already_synced {
        let existing = run.retro.clone().unwrap();
        let mut data = Value::object();
        data.insert("synced", true);
        data.insert("journalFile", existing.journal_file.as_str());
        data.insert("method", existing.method.as_str());
        data.insert("alreadySynced", true);
        return emit(
            &format!(
                "Already synced to {} - nothing to do.",
                existing.journal_file
            ),
            &data,
            &args.flags,
        );
    }

    let branch = current_branch(&ctx.root);
    let entry = format_journal_entry(&JournalEntryParams {
        run_id: &run.id,
        run_title: &run.title,
        run_profile: &run.profile,
        retro: &retro,
        branch: branch.as_deref(),
        when: Some(when),
    });
    let result = write_journal_entry(&ctx.root, &entry, Some(when))?;

    run.retro = Some(RetroSync {
        journal_file: result.journal_file.clone(),
        synced_at: now_iso(),
        method: match result.method {
            JournalWriteMethod::AgnosgramCli => RetroMethod::AgnosgramCli,
            JournalWriteMethod::Fallback => RetroMethod::Fallback,
        },
    });
    write_run(&ctx.root, &mut run).map_err(|e| UserError::new(e.to_string()))?;

    let mut data = Value::object();
    data.insert("synced", true);
    data.insert("journalFile", result.journal_file.as_str());
    data.insert("method", result.method.as_str());
    data.insert("alreadySynced", false);
    emit(
        &format!(
            "Synced retro to {} ({})",
            result.journal_file,
            result.method.as_str()
        ),
        &data,
        &args.flags,
    )
}
