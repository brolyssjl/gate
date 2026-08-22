//! Port of `src/core/playbooks.ts`: resolve the active playbook for a
//! phase and layer target overlays onto it.
//!
//! `find_bundled_playbooks_dir`'s marker: TS walks up from the *running*
//! JS file's own location looking for a directory that has both a
//! `package.json` and a sibling `playbooks/` (the npm-install and
//! dev-from-source layouts - a single-file Node SEA binary has neither, so
//! it falls through to the embedded copy). The Rust binary has no
//! `package.json` of its own, but `cargo run`/`cargo test` from within a
//! checkout of this repo still has one at the repo root, right alongside
//! `playbooks/` - reusing the identical marker lets the disk-walk fallback
//! work for dev-from-source in this repo exactly as it does for the TS
//! binary, and correctly find nothing (falling back to
//! `embedded_playbooks`) for an installed single-file release binary with
//! no repo checkout around it. Starting point: `std::env::current_exe()`
//! in place of `fileURLToPath(import.meta.url)`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::config::GateConfig;
use crate::core::embedded_playbooks::embedded_playbook;
use crate::core::paths::gate_paths;
use crate::core::state_machine::Phase;

fn find_bundled_playbooks_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    loop {
        let candidate = dir.join("playbooks");
        if dir.join("package.json").exists() && candidate.is_dir() {
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

/// Resolve the active playbook for a phase: the user's editable copy under
/// `.gate/playbooks/` wins; otherwise the bundled default on disk; otherwise
/// the copy compiled into the binary at build time. `None` for phases
/// without a playbook (e.g. DONE).
pub fn resolve_playbook(root: &Path, phase: Phase) -> Option<String> {
    let name = format!("{}.md", phase.as_str().to_lowercase());
    let user_path = gate_paths(root).playbooks.join(&name);
    if let Ok(content) = fs::read_to_string(&user_path) {
        return Some(content);
    }
    if let Some(dir) = find_bundled_playbooks_dir() {
        let bundled = dir.join(&name);
        if let Ok(content) = fs::read_to_string(&bundled) {
            return Some(content);
        }
    }
    embedded_playbook(&phase.as_str().to_lowercase()).map(String::from)
}

/// The base playbook for `phase`, with one `## Target overlay: <name>`
/// section appended per affected target that declares an overlay for this
/// phase in its `config.yml` `playbooks:` map. No affected targets, or none
/// with an overlay for this phase, returns the base playbook unchanged.
pub fn resolve_playbook_with_overlays(
    root: &Path,
    phase: Phase,
    config: &GateConfig,
    target_names: &[String],
) -> Option<String> {
    let base = resolve_playbook(root, phase)?;

    let phase_key = phase.as_str().to_lowercase();
    let mut overlays: Vec<String> = Vec::new();
    for name in target_names {
        let Some(target) = config.get_target(name) else {
            continue;
        };
        let Some((_, overlay_rel)) = target.playbooks.iter().find(|(k, _)| *k == phase_key) else {
            continue;
        };
        let overlay_path = root.join(overlay_rel);
        let Ok(content) = fs::read_to_string(&overlay_path) else {
            continue;
        };
        let content = content.trim();
        if !content.is_empty() {
            overlays.push(format!("## Target overlay: {name}\n\n{content}"));
        }
    }
    if overlays.is_empty() {
        return Some(base);
    }
    Some(format!(
        "{}\n\n{}\n",
        base.trim_end(),
        overlays.join("\n\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{Commands, TargetConfig, Thresholds};
    use std::fs as stdfs;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-playbooks-rs-{name}-{}", std::process::id()));
        let _ = stdfs::remove_dir_all(&dir);
        stdfs::create_dir_all(&dir).unwrap();
        dir
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
    fn a_user_override_under_dot_gate_playbooks_wins() {
        let root = tmp_dir("user-override");
        let dir = gate_paths(&root).playbooks;
        stdfs::create_dir_all(&dir).unwrap();
        stdfs::write(dir.join("plan.md"), "# custom plan\n").unwrap();
        assert_eq!(
            resolve_playbook(&root, Phase::Plan).unwrap(),
            "# custom plan\n"
        );
        stdfs::remove_dir_all(&root).unwrap();
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
        // cargo test runs from within this repo checkout, which has a
        // package.json + playbooks/ sibling at the repo root - the same
        // marker TS's own dev-from-source fallback uses.
        let dir =
            find_bundled_playbooks_dir().expect("expected to find playbooks/ during cargo test");
        assert!(dir.join("plan.md").exists());
    }

    #[test]
    fn appends_a_target_overlay_section_when_one_is_declared_and_present() {
        let root = tmp_dir("overlay");
        stdfs::create_dir_all(root.join(".gate")).unwrap();
        stdfs::write(root.join("overlay.md"), "Use pytest fixtures.\n").unwrap();
        let mut config = GateConfig::default();
        config.targets.push((
            "api".to_string(),
            TargetConfig {
                match_globs: vec!["apps/api/**".to_string()],
                commands: Commands::default(),
                thresholds: Thresholds::default(),
                playbooks: vec![("test".to_string(), "overlay.md".to_string())],
                coverage_format: None,
            },
        ));
        let result =
            resolve_playbook_with_overlays(&root, Phase::Test, &config, &["api".to_string()])
                .unwrap();
        assert!(result.contains("TEST playbook"));
        assert!(result.contains("## Target overlay: api"));
        assert!(result.contains("Use pytest fixtures."));
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
