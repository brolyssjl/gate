//! Resolve the active playbook for a phase and layer target overlays onto
//! it.
//!
//! `find_bundled_playbooks_dir`'s marker: walk up from the *running*
//! binary's own location looking for a directory that has both
//! `rust/Cargo.toml` and a sibling `playbooks/` - i.e. this repo's root.
//! `cargo run`/`cargo test` from within a checkout finds it (dev-from-source:
//! a playbook edit is live immediately, no rebuild needed); an installed
//! single-file release binary, copied out of any repo checkout, finds
//! neither and falls through to `embedded_playbooks`. Starting point:
//! `std::env::current_exe()`.
//!
//! `.gate/playbooks/*.md` overrides and target `playbooks:`
//! overlays are what gate *tells the agent* to execute, the same trust
//! class as `commands:` (`core/trust.rs` and `gates/implement.rs`'s
//! `command_check`, which this mirrors) - so both are folded into the TOFU
//! commands-block hash (`commands_block_hash_source`) and both refuse to
//! take effect while that hash is stale or absent, falling back to the
//! bundled/embedded default with a banner naming what was refused, instead
//! of silently running an unreviewed override.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::config::GateConfig;
use crate::core::embedded_playbooks::{embedded_playbook, EMBEDDED_PLAYBOOKS};
use crate::core::paths::gate_paths;
use crate::core::state_machine::Phase;
use crate::core::trust::is_commands_trusted;

fn untrusted_override_banner(what: &str) -> String {
    format!(
        "> **UNTRUSTED PLAYBOOK IGNORED**: {what} is not covered by the current \
trust hash (or nothing has been trusted yet). Falling back to the bundled \
default. Review the change, then run `gate trust`.\n\n"
    )
}

fn find_bundled_playbooks_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    loop {
        let candidate = dir.join("playbooks");
        if dir.join("rust").join("Cargo.toml").exists() && candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?.to_path_buf();
    }
}

/// Same walk, but `None` becomes an error message - for `gate init`, which
/// needs the real directory to enumerate.
pub fn bundled_playbooks_dir() -> Result<PathBuf, String> {
    find_bundled_playbooks_dir().ok_or_else(|| "bundled playbooks directory not found".to_string())
}

pub fn bundled_playbook_path(phase: Phase) -> Result<PathBuf, String> {
    Ok(bundled_playbooks_dir()?.join(format!("{}.md", phase.as_str().to_lowercase())))
}

/// Every bundled playbook file as `gate init`/`doctor`/`update` currently see
/// it: prefer the real `playbooks/` directory on disk (dev-from-source - a
/// playbook edit is live immediately), falling back to the copy compiled
/// into the binary at build time for an installed single-file release.
/// `(filename, content)` pairs, one per phase that has a playbook
/// (`core::embedded_playbooks::EMBEDDED_PLAYBOOKS`'s phase list). A real
/// `playbooks/` dir that exists but has an unreadable file surfaces on
/// stderr rather than going unmentioned; the embedded fallback (a fully
/// materialized crate-compiled copy) always succeeds, so this never fails
/// outright - "no bundled playbooks" isn't a state gate ever needs to
/// represent as an error.
pub fn current_bundled_playbooks() -> Vec<(String, String)> {
    match try_read_bundled_playbook_files() {
        Ok(files) => files,
        Err(e) => {
            let expected = e == "bundled playbooks directory not found";
            if !expected {
                eprintln!(
                    "gate: warning: could not read bundled playbooks ({e}) - using the built-in copy"
                );
            }
            EMBEDDED_PLAYBOOKS
                .iter()
                .map(|(phase, content)| (format!("{phase}.md"), (*content).to_string()))
                .collect()
        }
    }
}

fn try_read_bundled_playbook_files() -> Result<Vec<(String, String)>, String> {
    let bundled = bundled_playbooks_dir()?;
    let names: Vec<String> = fs::read_dir(&bundled)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|f| f.ends_with(".md"))
        .collect();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let content = fs::read_to_string(bundled.join(&name)).map_err(|e| e.to_string())?;
        out.push((name, content));
    }
    Ok(out)
}

fn bundled_or_embedded(name: &str, phase: Phase) -> Option<String> {
    if let Some(dir) = find_bundled_playbooks_dir() {
        let bundled = dir.join(name);
        if let Ok(content) = fs::read_to_string(&bundled) {
            return Some(content);
        }
    }
    embedded_playbook(&phase.as_str().to_lowercase()).map(String::from)
}

/// Resolve the active playbook for a phase: the user's editable copy under
/// `.gate/playbooks/` wins, PROVIDED the commands-block trust hash covers
/// it - an untrusted override is refused and the bundled default on
/// disk is used instead, falling back further to the copy compiled into the
/// binary at build time. `None` for phases without a playbook (e.g. DONE).
pub fn resolve_playbook(root: &Path, phase: Phase) -> Option<String> {
    let name = format!("{}.md", phase.as_str().to_lowercase());
    let user_path = gate_paths(root).playbooks.join(&name);
    if let Ok(content) = fs::read_to_string(&user_path) {
        if is_commands_trusted(root) {
            return Some(content);
        }
        let fallback = bundled_or_embedded(&name, phase)?;
        return Some(format!(
            "{}{}",
            untrusted_override_banner(&format!(".gate/playbooks/{name}")),
            fallback
        ));
    }
    bundled_or_embedded(&name, phase)
}

/// The base playbook for `phase`, with one `## Target overlay: <name>`
/// section appended per affected target that declares an overlay for this
/// phase in its `config.yml` `playbooks:` map, PROVIDED the commands-block
/// trust hash covers it - untrusted overlays are refused (skipped
/// entirely, with a banner naming which ones) rather than appended. No
/// affected targets, or none with an overlay for this phase, returns the
/// base playbook unchanged.
pub fn resolve_playbook_with_overlays(
    root: &Path,
    phase: Phase,
    config: &GateConfig,
    target_names: &[String],
) -> Option<String> {
    let base = resolve_playbook(root, phase)?;
    let trusted = is_commands_trusted(root);

    let phase_key = phase.as_str().to_lowercase();
    let mut overlays: Vec<String> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for name in target_names {
        let Some(target) = config.get_target(name) else {
            continue;
        };
        let Some((_, overlay_rel)) = target.playbooks.iter().find(|(k, _)| *k == phase_key) else {
            continue;
        };
        if !trusted {
            refused.push(format!("{name} ({overlay_rel})"));
            continue;
        }
        let overlay_path = root.join(overlay_rel);
        let Ok(content) = fs::read_to_string(&overlay_path) else {
            continue;
        };
        let content = content.trim();
        if !content.is_empty() {
            overlays.push(format!("## Target overlay: {name}\n\n{content}"));
        }
    }

    let mut result = base;
    if !refused.is_empty() {
        result = format!(
            "{}\n\n{}",
            result.trim_end(),
            untrusted_override_banner(&format!(
                "target playbook overlay(s) {}",
                refused.join(", ")
            ))
            .trim_end()
        );
    }
    if overlays.is_empty() {
        return Some(result);
    }
    Some(format!(
        "{}\n\n{}\n",
        result.trim_end(),
        overlays.join("\n\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::load_config;
    use crate::core::trust::write_trust;
    use std::fs as stdfs;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-playbooks-rs-{name}-{}", std::process::id()));
        let _ = stdfs::remove_dir_all(&dir);
        stdfs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_config(root: &Path, content: &str) {
        stdfs::create_dir_all(root.join(".gate")).unwrap();
        stdfs::write(root.join(".gate/config.yml"), content).unwrap();
    }

    #[test]
    fn resolves_to_the_embedded_copy_when_nothing_else_is_present() {
        let root = tmp_dir("embedded-fallback");
        stdfs::create_dir_all(root.join(".gate")).unwrap();
        let content = resolve_playbook(&root, Phase::Plan).unwrap();
        assert!(content.contains("PLAN playbook"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_trusted_user_override_under_dot_gate_playbooks_wins() {
        let root = tmp_dir("user-override-trusted");
        write_config(&root, "commands: {}\n");
        let dir = gate_paths(&root).playbooks;
        stdfs::create_dir_all(&dir).unwrap();
        stdfs::write(dir.join("plan.md"), "# custom plan\n").unwrap();
        write_trust(&root, None).unwrap();
        assert_eq!(
            resolve_playbook(&root, Phase::Plan).unwrap(),
            "# custom plan\n"
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    /// An override written after (or without ever) running `gate trust`
    /// must not take effect - it's refused, the bundled/embedded
    /// default is used instead, and a banner says so (so the refusal is
    /// visible wherever this content surfaces: `gate start`/`next`,
    /// `gate playbook`, the review packet).
    #[test]
    fn an_untrusted_user_override_is_refused_and_falls_back_to_the_embedded_default() {
        let root = tmp_dir("user-override-untrusted");
        write_config(&root, "commands: {}\n");
        let dir = gate_paths(&root).playbooks;
        stdfs::create_dir_all(&dir).unwrap();
        stdfs::write(dir.join("plan.md"), "# custom plan\n").unwrap();
        // No `write_trust` call: the override exists but was never approved.

        let result = resolve_playbook(&root, Phase::Plan).unwrap();
        assert!(result.contains("UNTRUSTED PLAYBOOK IGNORED"));
        assert!(result.contains(".gate/playbooks/plan.md"));
        assert!(result.contains("gate trust"));
        assert!(result.contains("PLAN playbook")); // the embedded default, not the override
        assert!(!result.contains("custom plan"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    /// Editing an already-trusted override without re-trusting must also be
    /// refused - trust is bound to content, not just to the override's
    /// existence.
    #[test]
    fn editing_a_trusted_override_without_re_trusting_is_refused() {
        let root = tmp_dir("user-override-edited-after-trust");
        write_config(&root, "commands: {}\n");
        let dir = gate_paths(&root).playbooks;
        stdfs::create_dir_all(&dir).unwrap();
        stdfs::write(dir.join("plan.md"), "# original\n").unwrap();
        write_trust(&root, None).unwrap();
        assert_eq!(
            resolve_playbook(&root, Phase::Plan).unwrap(),
            "# original\n"
        );

        stdfs::write(dir.join("plan.md"), "# tampered\n").unwrap();
        let result = resolve_playbook(&root, Phase::Plan).unwrap();
        assert!(result.contains("UNTRUSTED PLAYBOOK IGNORED"));
        assert!(!result.contains("tampered"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn current_bundled_playbooks_covers_every_embedded_phase() {
        let files = current_bundled_playbooks();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        for phase in ["plan", "debug", "implement", "test", "review", "retro"] {
            assert!(
                names.contains(&format!("{phase}.md").as_str()),
                "missing {phase}.md in {names:?}"
            );
        }
    }

    #[test]
    fn returns_none_for_a_phase_without_a_playbook() {
        let root = tmp_dir("no-playbook-phase");
        stdfs::create_dir_all(root.join(".gate")).unwrap();
        assert_eq!(resolve_playbook(&root, Phase::Done), None);
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn finds_the_real_on_disk_playbooks_dir_when_running_from_a_checkout_of_this_repo() {
        // cargo test runs from within this repo checkout, which has
        // rust/Cargo.toml + a playbooks/ sibling at the repo root.
        let dir =
            find_bundled_playbooks_dir().expect("expected to find playbooks/ during cargo test");
        assert!(dir.join("plan.md").exists());
    }

    #[test]
    fn appends_a_target_overlay_section_when_one_is_declared_trusted_and_present() {
        let root = tmp_dir("overlay-trusted");
        write_config(
            &root,
            "targets:\n  api:\n    match: [apps/api/**]\n    playbooks: { test: overlay.md }\n",
        );
        stdfs::write(root.join("overlay.md"), "Use pytest fixtures.\n").unwrap();
        write_trust(&root, None).unwrap();
        let config = load_config(&root).unwrap();

        let result =
            resolve_playbook_with_overlays(&root, Phase::Test, &config, &["api".to_string()])
                .unwrap();
        assert!(result.contains("TEST playbook"));
        assert!(result.contains("## Target overlay: api"));
        assert!(result.contains("Use pytest fixtures."));
        stdfs::remove_dir_all(&root).unwrap();
    }

    /// An overlay declared in config.yml but not covered by the
    /// current trust hash (never trusted, or trusted before the overlay was
    /// added/edited) is refused - skipped entirely, not appended - and the
    /// base playbook still comes back with a banner naming what was
    /// skipped, not an error.
    #[test]
    fn an_untrusted_target_overlay_is_refused_base_playbook_still_returned() {
        let root = tmp_dir("overlay-untrusted");
        write_config(
            &root,
            "targets:\n  api:\n    match: [apps/api/**]\n    playbooks: { test: overlay.md }\n",
        );
        stdfs::write(root.join("overlay.md"), "Use pytest fixtures.\n").unwrap();
        // No `write_trust` call.
        let config = load_config(&root).unwrap();

        let result =
            resolve_playbook_with_overlays(&root, Phase::Test, &config, &["api".to_string()])
                .unwrap();
        assert!(result.contains("TEST playbook"));
        assert!(result.contains("UNTRUSTED PLAYBOOK IGNORED"));
        assert!(result.contains("api"));
        assert!(!result.contains("## Target overlay: api"));
        assert!(!result.contains("Use pytest fixtures."));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn returns_the_base_playbook_unchanged_when_no_target_declares_an_overlay() {
        let root = tmp_dir("no-overlay");
        stdfs::create_dir_all(root.join(".gate")).unwrap();
        let config = GateConfig::default();
        let base = resolve_playbook(&root, Phase::Test).unwrap();
        let result = resolve_playbook_with_overlays(&root, Phase::Test, &config, &[]).unwrap();
        assert_eq!(result, base);
        stdfs::remove_dir_all(&root).unwrap();
    }
}
