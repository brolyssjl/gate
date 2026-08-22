//! Port of `src/gates/retro.ts` (`retroGate`). Ported in wave 3 (see
//! `docs/rust-port.md`).
//!
//! RETRO gate - deterministic:
//!  - retro.md exists and follows the schema (broke/avoid/conventions lists)
//!  - at least one of the three questions was actually answered
//!  - when an Agnosgram store is present and the integration is not "off",
//!    the entry was synced to the journal (`gate retro`); otherwise this
//!    check is advisory (pass-with-note - there is nowhere to sync to)
//!
//! Whether the retro is *insightful* is judgment and lives in the RETRO
//! playbook; the gate only checks that the ritual happened and, where a
//! store exists, that the journal actually received it.

use crate::artifacts::retro::{has_substance, parse_retro_file};
use crate::core::paths::run_paths;
use crate::core::state_machine::Phase;
use crate::gates::plan_drift::plan_drift_check;
use crate::gates::types::{fail, pass, result, Check, GateContext, GateResult};
use crate::integrations::agnosgram_write::journal_contains_run_id;

fn integration<'a>(config: &'a crate::core::config::GateConfig, key: &str) -> Option<&'a str> {
    config
        .integrations
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub fn retro_gate(ctx: &GateContext) -> GateResult {
    let mut checks: Vec<Check> = Vec::new();
    if let Some(drift) = plan_drift_check(ctx, "retro.plan-drift") {
        checks.push(drift);
    }
    let paths = run_paths(&ctx.root, &ctx.run.id);

    let parsed = parse_retro_file(&paths.retro);
    let Some(retro) = parsed.retro else {
        checks.push(fail(
            "retro.schema",
            format!("retro.md invalid: {}", parsed.errors.join("; ")),
        ));
        return result(Phase::Retro, checks);
    };
    checks.push(pass("retro.schema", "retro.md matches the required schema"));

    checks.push(if has_substance(&retro) {
        pass(
            "retro.substance",
            "at least one of broke/avoid/conventions was answered",
        )
    } else {
        fail(
            "retro.substance",
            "retro.md is empty - answer at least one of broke/avoid/conventions",
        )
    });

    checks.push(journal_check(ctx));

    result(Phase::Retro, checks)
}

fn journal_check(ctx: &GateContext) -> Check {
    if integration(&ctx.config, "agnosgram") == Some("off") {
        return pass(
            "retro.journal",
            "agnosgram integration is off - journal sync not required",
        );
    }
    if !ctx.root.join(".agnosgram").exists() {
        return pass(
            "retro.journal",
            "no .agnosgram store detected - journal sync not required",
        );
    }

    let Some(sync) = &ctx.run.retro else {
        return fail(
            "retro.journal",
            "no journal sync recorded - run `gate retro` to write the journal entry",
        );
    };
    if !ctx.root.join(&sync.journal_file).exists() {
        return fail(
            "retro.journal",
            format!(
                "journal file not found: {} - re-run `gate retro`",
                sync.journal_file
            ),
        );
    }
    if !journal_contains_run_id(&ctx.root, &sync.journal_file, &ctx.run.id) {
        return fail(
            "retro.journal",
            format!(
                "journal file {} does not contain run id \"{}\" - re-run `gate retro`",
                sync.journal_file, ctx.run.id
            ),
        );
    }
    pass(
        "retro.journal",
        format!("journal entry recorded in {}", sync.journal_file),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::GateConfig;
    use crate::core::run::{new_run, now_iso, NewRunParams, RetroMethod, RetroSync};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-retro-gate-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let abs = root.join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(abs, content).unwrap();
    }

    fn write_retro(root: &Path, content: &str) {
        write_file(root, ".gate/runs/r1/retro.md", content);
    }

    const SUBSTANTIVE_RETRO: &str =
        "---\nbroke: []\navoid:\n  - \"Do not skip reproduce\"\nconventions: []\n---\n# Retro";
    const EMPTY_RETRO: &str = "---\nbroke: []\navoid: []\nconventions: []\n---\n# Retro";

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
        run.phase = Phase::Retro;
        run
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
    fn fails_when_retro_md_is_missing() {
        let root = tmp_dir("missing");
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config: GateConfig::default(),
        };
        let res = retro_gate(&ctx);
        assert!(!check(&res, "retro.schema").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_substance_on_an_empty_scaffold() {
        let root = tmp_dir("empty-scaffold");
        write_retro(&root, EMPTY_RETRO);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config: GateConfig::default(),
        };
        let res = retro_gate(&ctx);
        assert!(!check(&res, "retro.substance").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn passes_with_no_agnosgram_store() {
        let root = tmp_dir("no-store");
        write_retro(&root, SUBSTANTIVE_RETRO);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config: GateConfig::default(),
        };
        let res = retro_gate(&ctx);
        assert!(res.ok, "{:?}", res.checks);
        assert!(check(&res, "retro.journal").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn passes_when_the_agnosgram_integration_is_off_even_with_a_store_present() {
        let root = tmp_dir("integration-off");
        write_file(&root, ".agnosgram/journal/2026-01.md", "# Journal\n");
        write_retro(&root, SUBSTANTIVE_RETRO);
        let config = write_config(&root, "integrations:\n  agnosgram: off\n");
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config,
        };
        let res = retro_gate(&ctx);
        assert!(check(&res, "retro.journal").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_retro_journal_when_a_store_is_present_but_no_sync_was_recorded() {
        let root = tmp_dir("store-no-sync");
        write_file(&root, ".agnosgram/journal/2026-01.md", "# Journal\n");
        write_retro(&root, SUBSTANTIVE_RETRO);
        let ctx = GateContext {
            root: root.clone(),
            run: run_on(),
            config: GateConfig::default(),
        };
        let res = retro_gate(&ctx);
        assert!(!check(&res, "retro.journal").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn passes_retro_journal_once_the_receipt_points_at_an_entry_containing_the_run_id() {
        let root = tmp_dir("sync-recorded");
        write_file(
            &root,
            ".agnosgram/journal/2026-01.md",
            "# Journal\n\n## entry\n- **Source:** .gate/runs/r1\n",
        );
        write_retro(&root, SUBSTANTIVE_RETRO);
        let mut run = run_on();
        run.retro = Some(RetroSync {
            journal_file: ".agnosgram/journal/2026-01.md".to_string(),
            synced_at: now_iso(),
            method: RetroMethod::Fallback,
        });
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = retro_gate(&ctx);
        assert!(res.ok, "{:?}", res.checks);
        assert!(check(&res, "retro.journal").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fails_retro_journal_when_the_recorded_file_no_longer_contains_the_run_id() {
        let root = tmp_dir("stale-sync");
        write_file(
            &root,
            ".agnosgram/journal/2026-01.md",
            "# Journal\n\n## entry\n- **Did:** unrelated\n",
        );
        write_retro(&root, SUBSTANTIVE_RETRO);
        let mut run = run_on();
        run.retro = Some(RetroSync {
            journal_file: ".agnosgram/journal/2026-01.md".to_string(),
            synced_at: now_iso(),
            method: RetroMethod::Fallback,
        });
        let ctx = GateContext {
            root: root.clone(),
            run,
            config: GateConfig::default(),
        };
        let res = retro_gate(&ctx);
        assert!(!check(&res, "retro.journal").unwrap().ok);
        fs::remove_dir_all(&root).unwrap();
    }
}
