//! Port of `src/core/targets.ts`: targets let a multi-stack repo declare
//! named per-stack blocks in `config.yml` (match globs, commands,
//! thresholds, playbook overlays). Composition rule (proposal §4.1):
//! **profiles choose which phases run; targets choose how each phase
//! runs.** Everything here is additive - a repo with no `targets:` block
//! resolves to a single bare, top-level command set, byte-identical to
//! Gate's behavior before targets existed.
//!
//! Adaptation note: TS functions take `Pick<Run, "targetOverride">` (a
//! duck-typed slice of the run) so they don't need the full `Run` type.
//! `core::run` isn't ported until wave 2, so this module takes the override
//! directly as `Option<&[String]>` instead - callers pass
//! `run.target_override.as_deref()` once `Run` exists. `resolveDisplayTargets`
//! (the one function that also calls `core::git::changed_files` and
//! `artifacts::plan::parse_plan_file`, both wave 2/3) is deferred to the
//! wave that ports those - see the module-level TODO at the bottom.

use crate::core::config::{Commands, CoverageFormat, GateConfig, Thresholds};
use crate::core::glob::matches_any;

/// Names of every target whose `match` globs cover at least one of `files`.
pub fn resolve_affected_targets(config: &GateConfig, files: &[String]) -> Vec<String> {
    config
        .targets
        .iter()
        .filter(|(_, target)| files.iter().any(|f| matches_any(f, &target.match_globs)))
        .map(|(name, _)| name.clone())
        .collect()
}

/// A target's effective commands: its own commands layered over the
/// top-level defaults.
pub fn target_commands(config: &GateConfig, name: &str) -> Commands {
    match config.get_target(name) {
        Some(t) => config.commands.layered_over(&t.commands),
        None => config.commands.clone(),
    }
}

/// A target's effective thresholds: its own thresholds layered over the
/// top-level defaults.
pub fn target_thresholds(config: &GateConfig, name: &str) -> Thresholds {
    match config.get_target(name) {
        Some(t) => config.thresholds.layered_over(&t.thresholds),
        None => config.thresholds,
    }
}

/// A target's effective coverage format: its own override, or the
/// top-level default.
pub fn target_coverage_format(config: &GateConfig, name: &str) -> CoverageFormat {
    config
        .get_target(name)
        .and_then(|t| t.coverage_format)
        .unwrap_or(config.coverage_format)
}

/// `base` for the untargeted case (JSON stability), `base[name]` once a
/// target applies.
pub fn check_name(base: &str, target: Option<&str>) -> String {
    match target {
        Some(t) => format!("{base}[{t}]"),
        None => base.to_string(),
    }
}

/// Files among `files` that fall under target `name`'s `match` globs.
pub fn files_for_target(config: &GateConfig, name: &str, files: &[String]) -> Vec<String> {
    match config.get_target(name) {
        Some(t) => files
            .iter()
            .filter(|f| matches_any(f, &t.match_globs))
            .cloned()
            .collect(),
        None => Vec::new(),
    }
}

/// The run's resolved targets for gate execution: an explicit `gate start
/// --target` override (stored in run.json) always wins over file-based
/// resolution. Unknown override names (a target since removed from config)
/// are dropped rather than silently treated as an active target.
pub fn resolve_run_targets(
    config: &GateConfig,
    target_override: Option<&[String]>,
    files: &[String],
) -> Vec<String> {
    match target_override {
        Some(names) if !names.is_empty() => names
            .iter()
            .filter(|n| config.get_target(n).is_some())
            .cloned()
            .collect(),
        _ => resolve_affected_targets(config, files),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTarget {
    /// `None` = no targets configured, or none affected - run the bare,
    /// top-level command set.
    pub target: Option<String>,
    pub commands: Commands,
    pub thresholds: Thresholds,
    pub coverage_format: CoverageFormat,
}

/// The command sets a gated phase should run this pass. Two cases collapse
/// to a single bare entry using the top-level commands/thresholds unchanged
/// (the critical invariant: no `targets:` configured, or none of `files`
/// matched any target, byte-identical to pre-targets behavior). Otherwise
/// one entry per affected target, each carrying its own effective
/// commands/thresholds and yielding bracketed check names via `check_name`.
pub fn resolve_phase_targets(
    config: &GateConfig,
    target_override: Option<&[String]>,
    files: &[String],
) -> Vec<ResolvedTarget> {
    let bare = ResolvedTarget {
        target: None,
        commands: config.commands.clone(),
        thresholds: config.thresholds,
        coverage_format: config.coverage_format,
    };
    if config.targets.is_empty() {
        return vec![bare];
    }
    let affected = resolve_run_targets(config, target_override, files);
    if affected.is_empty() {
        return vec![bare];
    }
    affected
        .into_iter()
        .map(|name| ResolvedTarget {
            commands: target_commands(config, &name),
            thresholds: target_thresholds(config, &name),
            coverage_format: target_coverage_format(config, &name),
            target: Some(name),
        })
        .collect()
}

/// The glob's literal prefix up to its first wildcard, directory-separator
/// trimmed.
fn static_prefix(glob: &str) -> &str {
    let idx = glob.find(['*', '?']).unwrap_or(glob.len());
    glob[..idx].trim_end_matches('/')
}

/// Whether a plan-declared file/glob entry could plausibly touch a target's
/// match glob, without a real diff to check against yet. Errs toward
/// inclusion (a wildcard-rooted glob on either side is treated as covering
/// everything; otherwise two globs overlap when one's static path prefix
/// contains the other's) - a false positive only shows an extra, harmless
/// playbook overlay.
fn globs_overlap(a: &str, b: &str) -> bool {
    let pa = static_prefix(a);
    let pb = static_prefix(b);
    if pa.is_empty() || pb.is_empty() {
        return true;
    }
    pa == pb || pa.starts_with(&format!("{pb}/")) || pb.starts_with(&format!("{pa}/"))
}

/// Targets whose match globs plausibly overlap any of a plan's declared
/// `files` entries.
pub fn resolve_targets_from_plan_files(config: &GateConfig, plan_files: &[String]) -> Vec<String> {
    config
        .targets
        .iter()
        .filter(|(_, target)| {
            plan_files
                .iter()
                .any(|pf| target.match_globs.iter().any(|g| globs_overlap(pf, g)))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

// `resolve_display_targets` (TS: best-effort target set for *display*
// purposes when no real diff exists yet, e.g. still in PLAN) is deferred:
// it calls `core::git::changed_files`/`is_gate_bookkeeping` and
// `artifacts::plan::parse_plan_file`, both ported in later waves. Add it
// here once those land, following TS's `resolveDisplayTargets` exactly:
//
//   pub fn resolve_display_targets(root: &Path, run: &Run, config: &GateConfig) -> Vec<String>

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::TargetConfig;

    fn empty_config() -> GateConfig {
        GateConfig {
            commands: Commands {
                test: Some("top-level-test".into()),
                build: Some("top-level-build".into()),
                ..Default::default()
            },
            thresholds: Thresholds {
                diff_coverage: Some(50.0),
            },
            ..Default::default()
        }
    }

    fn two_targets() -> GateConfig {
        let mut config = empty_config();
        config.targets = vec![
            (
                "api".to_string(),
                TargetConfig {
                    match_globs: vec!["apps/api/**".to_string()],
                    commands: Commands {
                        test: Some("pytest".into()),
                        ..Default::default()
                    },
                    thresholds: Thresholds {
                        diff_coverage: Some(85.0),
                    },
                    coverage_format: Some(CoverageFormat::CoveragePy),
                    ..Default::default()
                },
            ),
            (
                "web".to_string(),
                TargetConfig {
                    match_globs: vec!["apps/web/**".to_string()],
                    commands: Commands {
                        test: Some("vitest run".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ),
        ];
        config
    }

    fn files(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolve_affected_targets_returns_nothing_when_no_targets_are_configured() {
        assert_eq!(
            resolve_affected_targets(&empty_config(), &files(&["apps/api/foo.py"])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn resolve_affected_targets_returns_targets_whose_match_globs_cover_at_least_one_file() {
        assert_eq!(
            resolve_affected_targets(&two_targets(), &files(&["apps/api/foo.py"])),
            vec!["api".to_string()]
        );
        let mut both = resolve_affected_targets(
            &two_targets(),
            &files(&["apps/web/foo.ts", "apps/api/foo.py"]),
        );
        both.sort();
        assert_eq!(both, vec!["api".to_string(), "web".to_string()]);
    }

    #[test]
    fn resolve_affected_targets_returns_nothing_when_files_match_no_target() {
        assert_eq!(
            resolve_affected_targets(&two_targets(), &files(&["README.md"])),
            Vec::<String>::new()
        );
    }

    #[test]
    fn layers_a_targets_own_values_over_the_top_level_defaults() {
        let cfg = two_targets();
        assert_eq!(
            target_commands(&cfg, "api"),
            Commands {
                test: Some("pytest".into()),
                build: Some("top-level-build".into()),
                ..Default::default()
            }
        );
        assert_eq!(
            target_thresholds(&cfg, "api"),
            Thresholds {
                diff_coverage: Some(85.0)
            }
        );
        // web has no thresholds override - inherits the top-level value.
        assert_eq!(
            target_thresholds(&cfg, "web"),
            Thresholds {
                diff_coverage: Some(50.0)
            }
        );
    }

    #[test]
    fn returns_the_top_level_values_unchanged_for_an_unknown_target_name() {
        let cfg = two_targets();
        assert_eq!(target_commands(&cfg, "nope"), cfg.commands);
    }

    #[test]
    fn a_targets_own_coverage_format_wins_falls_back_without_one() {
        let cfg = two_targets();
        assert_eq!(
            target_coverage_format(&cfg, "api"),
            CoverageFormat::CoveragePy
        );
        assert_eq!(target_coverage_format(&cfg, "web"), CoverageFormat::Auto);
    }

    #[test]
    fn check_name_is_bare_for_no_target_and_bracketed_otherwise() {
        assert_eq!(check_name("test.command", None), "test.command");
        assert_eq!(check_name("test.command", Some("api")), "test.command[api]");
    }

    #[test]
    fn files_for_target_filters_to_files_under_the_targets_match_globs() {
        let cfg = two_targets();
        let f = files(&["apps/api/a.py", "apps/web/b.ts", "README.md"]);
        assert_eq!(
            files_for_target(&cfg, "api", &f),
            vec!["apps/api/a.py".to_string()]
        );
    }

    #[test]
    fn resolve_run_targets_prefers_an_explicit_override_over_file_based_resolution() {
        let cfg = two_targets();
        let over = vec!["web".to_string()];
        assert_eq!(
            resolve_run_targets(&cfg, Some(&over), &files(&["apps/api/a.py"])),
            vec!["web".to_string()]
        );
    }

    #[test]
    fn resolve_run_targets_drops_override_names_that_no_longer_exist_in_config() {
        let cfg = two_targets();
        let over = vec!["ghost".to_string(), "api".to_string()];
        assert_eq!(
            resolve_run_targets(&cfg, Some(&over), &[]),
            vec!["api".to_string()]
        );
    }

    #[test]
    fn resolve_run_targets_falls_back_to_file_based_resolution_with_no_override() {
        let cfg = two_targets();
        assert_eq!(
            resolve_run_targets(&cfg, None, &files(&["apps/web/x.ts"])),
            vec!["web".to_string()]
        );
    }

    #[test]
    fn resolve_phase_targets_collapses_to_a_bare_entry_when_no_targets_are_configured() {
        let cfg = empty_config();
        let resolved = resolve_phase_targets(&cfg, None, &files(&["src/x.ts"]));
        assert_eq!(
            resolved,
            vec![ResolvedTarget {
                target: None,
                commands: cfg.commands.clone(),
                thresholds: cfg.thresholds,
                coverage_format: CoverageFormat::Auto
            }]
        );
    }

    #[test]
    fn resolve_phase_targets_collapses_to_a_bare_entry_when_targets_configured_but_none_affected() {
        let cfg = two_targets();
        let resolved = resolve_phase_targets(&cfg, None, &files(&["README.md"]));
        assert_eq!(
            resolved,
            vec![ResolvedTarget {
                target: None,
                commands: cfg.commands.clone(),
                thresholds: cfg.thresholds,
                coverage_format: CoverageFormat::Auto
            }]
        );
    }

    #[test]
    fn resolve_phase_targets_returns_one_entry_per_affected_target() {
        let cfg = two_targets();
        let mut resolved =
            resolve_phase_targets(&cfg, None, &files(&["apps/api/a.py", "apps/web/b.ts"]));
        resolved.sort_by(|a, b| a.target.cmp(&b.target));
        let names: Vec<Option<String>> = resolved.iter().map(|r| r.target.clone()).collect();
        assert_eq!(
            names,
            vec![Some("api".to_string()), Some("web".to_string())]
        );
        let api = resolved
            .iter()
            .find(|r| r.target.as_deref() == Some("api"))
            .unwrap();
        assert_eq!(api.commands.test.as_deref(), Some("pytest"));
        assert_eq!(api.thresholds.diff_coverage, Some(85.0));
    }

    #[test]
    fn honors_a_target_override_even_when_no_files_match_it() {
        let cfg = two_targets();
        let over = vec!["api".to_string()];
        let resolved = resolve_phase_targets(&cfg, Some(&over), &[]);
        assert_eq!(
            resolved,
            vec![ResolvedTarget {
                target: Some("api".to_string()),
                commands: target_commands(&cfg, "api"),
                thresholds: target_thresholds(&cfg, "api"),
                coverage_format: CoverageFormat::CoveragePy,
            }]
        );
    }

    #[test]
    fn resolve_targets_from_plan_files_matches_a_plan_file_entry_against_target_match_globs() {
        let cfg = two_targets();
        assert_eq!(
            resolve_targets_from_plan_files(&cfg, &files(&["apps/api/**"])),
            vec!["api".to_string()]
        );
    }

    #[test]
    fn resolve_targets_from_plan_files_errs_toward_inclusion_for_a_bare_wildcard_plan_entry() {
        let cfg = two_targets();
        let mut both = resolve_targets_from_plan_files(&cfg, &files(&["**"]));
        both.sort();
        assert_eq!(both, vec!["api".to_string(), "web".to_string()]);
    }

    #[test]
    fn resolve_targets_from_plan_files_finds_no_overlap_for_a_plan_entry_outside_every_targets_directory(
    ) {
        let cfg = two_targets();
        assert_eq!(
            resolve_targets_from_plan_files(&cfg, &files(&["docs/readme.md"])),
            Vec::<String>::new()
        );
    }
}
