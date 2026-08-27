//! Port of `src/core/config.ts`: `.gate/config.yml` loading and validation.

use std::fs;
use std::path::Path;

use crate::cli::output::UserError;
use crate::core::json::{self, Value as JsonValue};
use crate::core::paths::gate_paths;
use crate::core::yaml::{self, YamlValue};

/// Machine actions per phase. Missing commands mean "no such action here".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commands {
    pub build: Option<String>,
    pub test: Option<String>,
    pub lint: Option<String>,
    pub coverage: Option<String>,
}

impl Commands {
    /// `{ ...base, ...override }`: `other`'s fields win where present.
    pub fn layered_over(&self, other: &Commands) -> Commands {
        Commands {
            build: other.build.clone().or_else(|| self.build.clone()),
            test: other.test.clone().or_else(|| self.test.clone()),
            lint: other.lint.clone().or_else(|| self.lint.clone()),
            coverage: other.coverage.clone().or_else(|| self.coverage.clone()),
        }
    }
}

/// Minimum percentage of changed lines that must be covered.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Thresholds {
    pub diff_coverage: Option<f64>,
}

impl Thresholds {
    pub fn layered_over(&self, other: &Thresholds) -> Thresholds {
        Thresholds {
            diff_coverage: other.diff_coverage.or(self.diff_coverage),
        }
    }
}

/// Diff-coverage report formats Gate can parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageFormat {
    Istanbul,
    Generic,
    CoveragePy,
    GoCover,
    Lcov,
    Auto,
}

const COVERAGE_FORMATS: &[&str] = &[
    "istanbul",
    "generic",
    "coverage-py",
    "go-cover",
    "lcov",
    "auto",
];

impl CoverageFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            CoverageFormat::Istanbul => "istanbul",
            CoverageFormat::Generic => "generic",
            CoverageFormat::CoveragePy => "coverage-py",
            CoverageFormat::GoCover => "go-cover",
            CoverageFormat::Lcov => "lcov",
            CoverageFormat::Auto => "auto",
        }
    }

    pub fn from_str_opt(value: &str) -> Option<CoverageFormat> {
        match value {
            "istanbul" => Some(CoverageFormat::Istanbul),
            "generic" => Some(CoverageFormat::Generic),
            "coverage-py" => Some(CoverageFormat::CoveragePy),
            "go-cover" => Some(CoverageFormat::GoCover),
            "lcov" => Some(CoverageFormat::Lcov),
            "auto" => Some(CoverageFormat::Auto),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetConfig {
    pub match_globs: Vec<String>,
    pub commands: Commands,
    pub thresholds: Thresholds,
    pub playbooks: Vec<(String, String)>,
    /// Overrides the top-level `coverage_format` for this target only.
    pub coverage_format: Option<CoverageFormat>,
}

/// `gate prune` retention defaults (Milestone 3, additive). CLI flags win
/// when given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetentionConfig {
    /// Keep the N most recently updated non-active runs (default 10).
    pub keep: Option<i64>,
    /// Additionally require a candidate to be older than N days to be pruned.
    pub days: Option<i64>,
}

/// Default `thresholds.failure_streak_limit` (the loop-enforcement cap)
/// when the key is absent from `config.yml`.
pub const DEFAULT_FAILURE_STREAK_LIMIT: i64 = 3;

/// `integrations.sdd_mapping:` overrides - per-field replacement of the
/// built-in phase-equivalence mapping (`integrations::sdd_mapping`) for
/// whichever SDD is actually detected (there is only ever one per repo).
/// `None` means the key was absent, so the built-in value stands; `Some("")`
/// is an explicit override to "gate-only"/"none"; any other `Some(s)`
/// replaces the built-in value outright. Advisory content only (it changes
/// hint text, never gate pass/fail), so - like the rest of `integrations:` -
/// it is deliberately outside the trust hash.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SddMappingOverride {
    pub plan: Option<String>,
    pub debug: Option<String>,
    pub implement: Option<String>,
    pub test: Option<String>,
    pub review: Option<String>,
    pub retro: Option<String>,
    pub plan_approval: Option<String>,
    pub closing_step: Option<String>,
    pub closing_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GateConfig {
    pub commands: Commands,
    pub thresholds: Thresholds,
    pub targets: Vec<(String, TargetConfig)>,
    pub phases: Vec<(String, String)>,
    pub integrations: Vec<(String, String)>,
    /// Coverage report format hint. Defaults to auto-detect.
    pub coverage_format: CoverageFormat,
    /// `gate prune` retention defaults. Deliberately NOT part of the trust
    /// hash (`commands_block_hash_source`) - it configures which run
    /// folders get archived, never a command that executes.
    pub retention: RetentionConfig,
    /// Glob list of paths the scope check (IMPLEMENT/DEBUG) treats as noise
    /// rather than an undeclared file. Part of the trust hash: it changes
    /// what the scope check accepts, so widening it needs the same
    /// re-trust as changing a command.
    pub scope_ignore: Vec<String>,
    /// `thresholds.failure_streak_limit` from `config.yml`, raw (pre
    /// default/disable resolution - use `failure_streak_cap`). `None` when
    /// unset. Top-level only, deliberately not part of `Thresholds` (which
    /// is shared with per-target overrides): the streak is a property of a
    /// run's phase, not of any one target, so a per-target value would
    /// silently do nothing.
    pub failure_streak_limit: Option<i64>,
    /// `integrations.sdd_mapping:` overrides - see `SddMappingOverride`.
    pub sdd_mapping_override: SddMappingOverride,
}

impl GateConfig {
    pub fn get_target(&self, name: &str) -> Option<&TargetConfig> {
        self.targets.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }

    /// Effective failure-streak cap: unset -> the default (3);
    /// explicit `0` -> disabled (`None`, no cap ever blocks); any other
    /// explicit value -> that cap. `0` is otherwise a meaningless cap (the
    /// streak starts at 0, so a cap of 0 would block before any evaluation
    /// ever ran) - repurposing it as the "disable" sentinel needs no new
    /// YAML syntax and mirrors the `--keep 0`/`0` = "unlimited" convention
    /// common to CLI retention knobs.
    pub fn failure_streak_cap(&self) -> Option<i64> {
        match self.failure_streak_limit {
            None => Some(DEFAULT_FAILURE_STREAK_LIMIT),
            Some(0) => None,
            Some(n) => Some(n),
        }
    }
}

impl Default for GateConfig {
    fn default() -> Self {
        GateConfig {
            commands: Commands::default(),
            thresholds: Thresholds::default(),
            targets: Vec::new(),
            phases: Vec::new(),
            integrations: Vec::new(),
            coverage_format: CoverageFormat::Auto,
            retention: RetentionConfig::default(),
            scope_ignore: Vec::new(),
            failure_streak_limit: None,
            sdd_mapping_override: SddMappingOverride::default(),
        }
    }
}

fn as_string_map(value: Option<&YamlValue>) -> Vec<(String, String)> {
    let Some(YamlValue::Map(entries)) = value else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn extract_commands(value: Option<&YamlValue>) -> Commands {
    let Some(YamlValue::Map(entries)) = value else {
        return Commands::default();
    };
    let get = |key: &str| {
        entries
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
    };
    Commands {
        build: get("build"),
        test: get("test"),
        lint: get("lint"),
        coverage: get("coverage"),
    }
}

fn extract_thresholds(value: Option<&YamlValue>) -> Thresholds {
    let Some(YamlValue::Map(entries)) = value else {
        return Thresholds::default();
    };
    let diff_coverage = entries
        .iter()
        .find(|(k, _)| k == "diff_coverage")
        .and_then(|(_, v)| v.as_f64());
    Thresholds { diff_coverage }
}

fn extract_retention(value: Option<&YamlValue>) -> RetentionConfig {
    let Some(YamlValue::Map(entries)) = value else {
        return RetentionConfig::default();
    };
    let get = |key: &str| {
        entries
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_i64())
    };
    RetentionConfig {
        keep: get("keep"),
        days: get("days"),
    }
}

fn extract_string_array(value: Option<&YamlValue>) -> Option<Vec<String>> {
    match value {
        Some(YamlValue::Array(items)) => Some(
            items
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
        ),
        _ => None,
    }
}

/// A malformed target block (most dangerously a missing/empty `match`) used
/// to reach the gates and blow up deep inside glob matching. Fail fast and
/// friendly here instead, naming the offending target so the fix is
/// obvious.
fn validate_and_extract_targets(
    raw_targets: Option<&YamlValue>,
) -> Result<Vec<(String, TargetConfig)>, UserError> {
    let Some(YamlValue::Map(entries)) = raw_targets else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(entries.len());
    for (name, raw) in entries {
        let YamlValue::Map(fields) = raw else {
            return Err(UserError::new(format!(
                ".gate/config.yml: target \"{name}\" must be a mapping"
            )));
        };
        let field = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);

        let match_globs = match field("match") {
            Some(YamlValue::Array(items))
                if !items.is_empty()
                    && items
                        .iter()
                        .all(|v| matches!(v.as_str(), Some(s) if !s.trim().is_empty())) =>
            {
                items
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect::<Vec<_>>()
            }
            _ => {
                return Err(UserError::new(format!(
                    ".gate/config.yml: target \"{name}\" must declare a non-empty `match` list of glob strings"
                )));
            }
        };

        if let Some(v) = field("commands") {
            if !matches!(v, YamlValue::Map(_)) {
                return Err(UserError::new(format!(
                    ".gate/config.yml: target \"{name}\".commands must be a mapping"
                )));
            }
        }
        if let Some(v) = field("thresholds") {
            if !matches!(v, YamlValue::Map(_)) {
                return Err(UserError::new(format!(
                    ".gate/config.yml: target \"{name}\".thresholds must be a mapping"
                )));
            }
        }
        if let Some(v) = field("playbooks") {
            if !matches!(v, YamlValue::Map(_)) {
                return Err(UserError::new(format!(
                    ".gate/config.yml: target \"{name}\".playbooks must be a mapping of phase -> path"
                )));
            }
        }
        let coverage_format = match field("coverage_format") {
            None => None,
            Some(v) => {
                let s = v.as_str().unwrap_or("");
                match CoverageFormat::from_str_opt(s) {
                    Some(cf) => Some(cf),
                    None => {
                        return Err(UserError::new(format!(
                            ".gate/config.yml: target \"{name}\".coverage_format must be one of {}",
                            COVERAGE_FORMATS.join(", ")
                        )));
                    }
                }
            }
        };

        out.push((
            name.clone(),
            TargetConfig {
                match_globs,
                commands: extract_commands(field("commands")),
                thresholds: extract_thresholds(field("thresholds")),
                playbooks: as_string_map(field("playbooks")),
                coverage_format,
            },
        ));
    }
    Ok(out)
}

fn read_raw_config(root: &Path) -> Result<Option<YamlValue>, UserError> {
    let config_path = gate_paths(root).config;
    if !config_path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&config_path).map_err(|e| UserError::new(e.to_string()))?;
    let parsed = yaml::parse_yaml(&text).map_err(|e| UserError::new(e.to_string()))?;
    Ok(Some(parsed))
}

pub fn load_config(root: &Path) -> Result<GateConfig, UserError> {
    let Some(raw) = read_raw_config(root)? else {
        return Ok(GateConfig::default());
    };
    let YamlValue::Map(_) = &raw else {
        return Ok(GateConfig::default());
    };

    let targets = validate_and_extract_targets(raw.get("targets"))?;

    let coverage_format = match raw.get("coverage_format") {
        None => CoverageFormat::Auto,
        Some(v) => {
            let s = v.as_str().unwrap_or("");
            CoverageFormat::from_str_opt(s).ok_or_else(|| {
                UserError::new(format!(
                    ".gate/config.yml: coverage_format must be one of {}",
                    COVERAGE_FORMATS.join(", ")
                ))
            })?
        }
    };

    let scope_ignore = match raw.get("scope_ignore") {
        None => Vec::new(),
        Some(v) => {
            let ok = matches!(v, YamlValue::Array(items) if items.iter().all(|item| matches!(item.as_str(), Some(s) if !s.trim().is_empty())));
            if !ok {
                return Err(UserError::new(
                    ".gate/config.yml: scope_ignore must be a list of glob strings",
                ));
            }
            extract_string_array(Some(v)).unwrap_or_default()
        }
    };

    let failure_streak_limit = extract_failure_streak_limit(raw.get("thresholds"))?;
    let sdd_mapping_override = extract_sdd_mapping_override(raw.get("integrations"));

    Ok(GateConfig {
        commands: extract_commands(raw.get("commands")),
        thresholds: extract_thresholds(raw.get("thresholds")),
        targets,
        phases: as_string_map(raw.get("phases")),
        integrations: as_string_map(raw.get("integrations")),
        coverage_format,
        retention: extract_retention(raw.get("retention")),
        scope_ignore,
        failure_streak_limit,
        sdd_mapping_override,
    })
}

/// `integrations.sdd_mapping:` - every field optional; an absent or
/// non-mapping `integrations:`/`sdd_mapping:` block yields every field
/// `None` (no overrides), the same "just use the built-in mapping" default
/// as a repo that predates this key entirely.
fn extract_sdd_mapping_override(integrations: Option<&YamlValue>) -> SddMappingOverride {
    let get = |key: &str| -> Option<String> {
        integrations?
            .get("sdd_mapping")?
            .get(key)?
            .as_str()
            .map(String::from)
    };
    SddMappingOverride {
        plan: get("plan"),
        debug: get("debug"),
        implement: get("implement"),
        test: get("test"),
        review: get("review"),
        retro: get("retro"),
        plan_approval: get("plan_approval"),
        closing_step: get("closing_step"),
        closing_command: get("closing_command"),
    }
}

/// `thresholds.failure_streak_limit` must be a non-negative integer when
/// present - `0` is the explicit "disabled" sentinel (see
/// `GateConfig::failure_streak_cap`), anything negative is a config error
/// naming the fix rather than silently misbehaving.
fn extract_failure_streak_limit(value: Option<&YamlValue>) -> Result<Option<i64>, UserError> {
    let Some(YamlValue::Map(entries)) = value else {
        return Ok(None);
    };
    let Some((_, raw)) = entries.iter().find(|(k, _)| k == "failure_streak_limit") else {
        return Ok(None);
    };
    match raw.as_i64() {
        Some(n) if n >= 0 => Ok(Some(n)),
        _ => Err(UserError::new(
            ".gate/config.yml: thresholds.failure_streak_limit must be a non-negative integer (0 disables the cap)",
        )),
    }
}

fn commands_to_json(commands: &Commands) -> JsonValue {
    let mut obj = JsonValue::object();
    if let Some(v) = &commands.build {
        obj.insert("build", v.as_str());
    }
    if let Some(v) = &commands.test {
        obj.insert("test", v.as_str());
    }
    if let Some(v) = &commands.lint {
        obj.insert("lint", v.as_str());
    }
    if let Some(v) = &commands.coverage {
        obj.insert("coverage", v.as_str());
    }
    obj
}

fn targets_to_json(root: &Path, targets: &[(String, TargetConfig)]) -> JsonValue {
    let mut obj = JsonValue::object();
    for (name, t) in targets {
        let mut entry = JsonValue::object();
        entry.insert("match", t.match_globs.clone());
        entry.insert("commands", commands_to_json(&t.commands));
        if !t.playbooks.is_empty() {
            entry.insert("playbooks", target_playbooks_to_json(root, &t.playbooks));
        }
        obj.insert(name.as_str(), entry);
    }
    obj
}

/// Per-target `playbooks:` overlay map, keyed by phase, each entry carrying
/// both its declared relative path AND that file's current content.
/// Hashing the path alone would let an attacker repoint an already-trusted
/// path at different content without invalidating trust.
fn target_playbooks_to_json(root: &Path, playbooks: &[(String, String)]) -> JsonValue {
    let mut obj = JsonValue::object();
    for (phase, rel_path) in playbooks {
        let mut entry = JsonValue::object();
        entry.insert("path", rel_path.as_str());
        entry.insert(
            "content",
            fs::read_to_string(root.join(rel_path)).unwrap_or_default(),
        );
        obj.insert(phase.as_str(), entry);
    }
    obj
}

/// `.gate/playbooks/*.md` overrides, sorted by filename, each paired with
/// its current content - the other half of the trust-hash coverage (target
/// overlays are `target_playbooks_to_json`'s job). A file that can't be read
/// (permissions, race) is silently omitted from hashing the same way a
/// missing file omits a target overlay's content above; `resolve_playbook`
/// itself hits the same read and has the same fallback.
fn playbook_override_entries(root: &Path) -> Vec<(String, String)> {
    let dir = gate_paths(root).playbooks;
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|f| f.ends_with(".md"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let content = fs::read_to_string(dir.join(&name)).ok()?;
            Some((name, content))
        })
        .collect()
}

/// Relative paths of every playbook file currently folded into the trust
/// hash - `.gate/playbooks/*.md` overrides and each target's `playbooks:`
/// overlay files - for `gate trust` to report what it is approving.
/// A malformed `targets:` block just yields no overlay paths here; loading
/// the config for real surfaces that error elsewhere.
pub fn trusted_playbook_paths(root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = playbook_override_entries(root)
        .into_iter()
        .map(|(name, _)| format!(".gate/playbooks/{name}"))
        .collect();

    if let Ok(Some(raw)) = read_raw_config(root) {
        if let Ok(targets) = validate_and_extract_targets(raw.get("targets")) {
            for (_, t) in &targets {
                for (_, rel_path) in &t.playbooks {
                    paths.push(rel_path.clone());
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn targets_to_json_legacy(targets: &[(String, TargetConfig)]) -> JsonValue {
    let mut obj = JsonValue::object();
    for (name, t) in targets {
        let mut entry = JsonValue::object();
        entry.insert("match", t.match_globs.clone());
        entry.insert("commands", commands_to_json(&t.commands));
        obj.insert(name.as_str(), entry);
    }
    obj
}

/// The pre-SEC-01 shape of `commands_block_hash_source` (trust coverage
/// version 1): commands + targets (without each target's `playbooks:`
/// overlay) + `scope_ignore` when non-empty - no playbook coverage at all.
/// Exists only so `core/trust.rs` can tell a real config edit apart from
/// gate's own coverage-widening upgrades on a `trust.json` written before
/// SEC-01: if a stored hash matches this legacy recomputation, the
/// pre-existing trust surface is genuinely unchanged and only gate's
/// coverage grew.
pub fn commands_block_hash_source_legacy(root: &Path) -> String {
    let config_path = gate_paths(root).config;
    if !config_path.exists() {
        return String::new();
    }
    let raw = fs::read_to_string(&config_path)
        .ok()
        .and_then(|text| yaml::parse_yaml(&text).ok())
        .unwrap_or(YamlValue::Null);

    let commands = extract_commands(raw.get("commands"));
    let targets = validate_and_extract_targets(raw.get("targets")).unwrap_or_default();
    let scope_ignore = extract_string_array(raw.get("scope_ignore")).unwrap_or_default();

    let mut source = JsonValue::object();
    source.insert("commands", commands_to_json(&commands));
    source.insert("targets", targets_to_json_legacy(&targets));
    if !scope_ignore.is_empty() {
        source.insert("scope_ignore", scope_ignore);
    }
    json::stringify_compact(&source)
}

/// Raw commands-block text, used by TOFU trust hashing (`core/trust.rs`,
/// wave 2). `scope_ignore` rides in the same hash: it changes what the
/// scope check accepts as noise rather than an undeclared file, the same
/// trust class as a command. Playbooks ride in it too: gate executes
/// `commands:` directly, and *tells the agent* to execute `playbooks:` -
/// both are instructions an attacker could plant, so both need a human's
/// review before they take effect.
///
/// `scope_ignore` and the two playbook sources are included only when
/// non-empty: a repo upgrading from a config with none of these has no way
/// to have set them, and the empty/absent default must hash identically to
/// the pre-existing shape (`{ commands, targets }`) or every existing
/// `trust.json` on disk would silently go stale the moment `gate` is
/// upgraded. A repo that
/// already has non-empty playbook overrides/overlays, however, is expected
/// to go stale on upgrade - that coverage is the point of this change.
pub fn commands_block_hash_source(root: &Path) -> String {
    let config_path = gate_paths(root).config;
    if !config_path.exists() {
        return String::new();
    }
    let raw = fs::read_to_string(&config_path)
        .ok()
        .and_then(|text| yaml::parse_yaml(&text).ok())
        .unwrap_or(YamlValue::Null);

    let commands = extract_commands(raw.get("commands"));
    let targets = validate_and_extract_targets(raw.get("targets")).unwrap_or_default();
    let scope_ignore = extract_string_array(raw.get("scope_ignore")).unwrap_or_default();
    let playbook_overrides = playbook_override_entries(root);

    let mut source = JsonValue::object();
    source.insert("commands", commands_to_json(&commands));
    source.insert("targets", targets_to_json(root, &targets));
    if !scope_ignore.is_empty() {
        source.insert("scope_ignore", scope_ignore);
    }
    if !playbook_overrides.is_empty() {
        let mut overrides_obj = JsonValue::object();
        for (name, content) in &playbook_overrides {
            overrides_obj.insert(name.as_str(), content.as_str());
        }
        source.insert("playbookOverrides", overrides_obj);
    }
    json::stringify_compact(&source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gate-config-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_config(root: &Path, content: &str) {
        fs::create_dir_all(root.join(".gate")).unwrap();
        fs::write(root.join(".gate/config.yml"), content).unwrap();
    }

    #[test]
    fn loads_a_well_formed_targets_block_unchanged() {
        let root = tmp_dir("targets-ok");
        write_config(&root, "targets:\n  api:\n    match:\n      - apps/api/**\n    commands:\n      test: pytest\n");
        let config = load_config(&root).unwrap();
        assert_eq!(
            config.get_target("api").unwrap().match_globs,
            vec!["apps/api/**".to_string()]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn throws_a_friendly_error_naming_the_target_when_match_is_missing() {
        let root = tmp_dir("targets-missing-match");
        write_config(
            &root,
            "targets:\n  api:\n    commands:\n      test: pytest\n",
        );
        let err = load_config(&root).unwrap_err();
        assert!(err.message().contains("\"api\""));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn throws_when_match_is_present_but_empty() {
        let root = tmp_dir("targets-empty-match");
        write_config(&root, "targets:\n  api:\n    match: []\n");
        assert!(load_config(&root).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn throws_when_match_contains_a_non_string_entry() {
        let root = tmp_dir("targets-non-string-match");
        write_config(&root, "targets:\n  api:\n    match:\n      - 5\n");
        assert!(load_config(&root).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn throws_when_a_targets_commands_block_is_not_a_mapping() {
        let root = tmp_dir("targets-bad-commands");
        write_config(
            &root,
            "targets:\n  api:\n    match: [apps/api/**]\n    commands: nope\n",
        );
        assert!(load_config(&root).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scope_ignore_defaults_to_an_empty_list() {
        let root = tmp_dir("scope-ignore-default");
        write_config(&root, "commands: {}\n");
        assert_eq!(
            load_config(&root).unwrap().scope_ignore,
            Vec::<String>::new()
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn loads_a_well_formed_scope_ignore_glob_list() {
        let root = tmp_dir("scope-ignore-ok");
        write_config(
            &root,
            "scope_ignore:\n  - \"node_modules/**\"\n  - \"coverage/**\"\n",
        );
        assert_eq!(
            load_config(&root).unwrap().scope_ignore,
            vec!["node_modules/**".to_string(), "coverage/**".to_string()]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn throws_when_scope_ignore_is_not_a_list_of_strings() {
        let root = tmp_dir("scope-ignore-not-list");
        write_config(&root, "scope_ignore: not-a-list\n");
        assert!(load_config(&root).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn throws_when_scope_ignore_contains_a_non_string_entry() {
        let root = tmp_dir("scope-ignore-non-string");
        write_config(&root, "scope_ignore:\n  - 5\n");
        assert!(load_config(&root).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failure_streak_limit_defaults_to_none_and_cap_defaults_to_three() {
        let root = tmp_dir("streak-limit-default");
        write_config(&root, "commands: {}\n");
        let config = load_config(&root).unwrap();
        assert_eq!(config.failure_streak_limit, None);
        assert_eq!(config.failure_streak_cap(), Some(3));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failure_streak_limit_loads_an_explicit_value() {
        let root = tmp_dir("streak-limit-explicit");
        write_config(&root, "thresholds:\n  failure_streak_limit: 5\n");
        let config = load_config(&root).unwrap();
        assert_eq!(config.failure_streak_limit, Some(5));
        assert_eq!(config.failure_streak_cap(), Some(5));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failure_streak_limit_zero_disables_the_cap() {
        let root = tmp_dir("streak-limit-disabled");
        write_config(&root, "thresholds:\n  failure_streak_limit: 0\n");
        let config = load_config(&root).unwrap();
        assert_eq!(config.failure_streak_limit, Some(0));
        assert_eq!(config.failure_streak_cap(), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failure_streak_limit_rejects_a_negative_value() {
        let root = tmp_dir("streak-limit-negative");
        write_config(&root, "thresholds:\n  failure_streak_limit: -1\n");
        let err = load_config(&root).unwrap_err();
        assert!(err.message().contains("failure_streak_limit"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failure_streak_limit_rejects_a_non_integer_value() {
        let root = tmp_dir("streak-limit-non-integer");
        write_config(&root, "thresholds:\n  failure_streak_limit: \"soon\"\n");
        assert!(load_config(&root).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn sdd_mapping_override_defaults_to_no_overrides() {
        let root = tmp_dir("sdd-mapping-default");
        write_config(&root, "commands: {}\n");
        assert_eq!(
            load_config(&root).unwrap().sdd_mapping_override,
            SddMappingOverride::default()
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn sdd_mapping_override_loads_declared_fields_only() {
        let root = tmp_dir("sdd-mapping-loaded");
        write_config(
            &root,
            "integrations:\n  sdd_mapping:\n    plan: draft\n    test: \"\"\n    closing_command: \"myddl close <change>\"\n",
        );
        let overrides = load_config(&root).unwrap().sdd_mapping_override;
        assert_eq!(overrides.plan.as_deref(), Some("draft"));
        assert_eq!(overrides.test.as_deref(), Some(""));
        assert_eq!(
            overrides.closing_command.as_deref(),
            Some("myddl close <change>")
        );
        assert_eq!(overrides.implement, None);
        assert_eq!(overrides.closing_step, None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_config_file_returns_defaults() {
        let root = tmp_dir("no-config");
        fs::create_dir_all(root.join(".gate")).unwrap();
        let config = load_config(&root).unwrap();
        assert_eq!(config, GateConfig::default());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn commands_block_hash_source_is_stable_and_excludes_retention() {
        let root = tmp_dir("hash-source");
        write_config(&root, "commands:\n  test: echo hi\nretention:\n  keep: 3\n");
        let source = commands_block_hash_source(&root);
        assert!(source.contains("echo hi"));
        assert!(!source.contains("retention"));
        assert!(!source.contains("keep"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn commands_block_hash_source_omits_scope_ignore_when_empty_for_backward_compatible_hashing() {
        let root = tmp_dir("hash-source-empty-scope");
        write_config(&root, "commands:\n  test: echo hi\n");
        let source = commands_block_hash_source(&root);
        assert!(!source.contains("scope_ignore"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn commands_block_hash_source_includes_scope_ignore_once_populated() {
        let root = tmp_dir("hash-source-with-scope");
        write_config(
            &root,
            "commands:\n  test: echo hi\nscope_ignore:\n  - dist/**\n",
        );
        let source = commands_block_hash_source(&root);
        assert!(source.contains("scope_ignore"));
        assert!(source.contains("dist/**"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn commands_block_hash_source_omits_playbooks_when_none_present_for_backward_compatible_hashing(
    ) {
        let root = tmp_dir("hash-source-no-playbooks");
        write_config(&root, "commands:\n  test: echo hi\n");
        let source = commands_block_hash_source(&root);
        assert_eq!(
            source,
            "{\"commands\":{\"test\":\"echo hi\"},\"targets\":{}}"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn commands_block_hash_source_changes_when_a_dot_gate_playbooks_override_is_added_or_edited() {
        let root = tmp_dir("hash-source-playbook-override");
        write_config(&root, "commands:\n  test: echo hi\n");
        let before = commands_block_hash_source(&root);

        fs::create_dir_all(root.join(".gate/playbooks")).unwrap();
        fs::write(root.join(".gate/playbooks/plan.md"), "# custom plan\n").unwrap();
        let with_override = commands_block_hash_source(&root);
        assert_ne!(before, with_override);
        assert!(with_override.contains("plan.md"));
        assert!(with_override.contains("custom plan"));

        fs::write(root.join(".gate/playbooks/plan.md"), "# edited plan\n").unwrap();
        let edited = commands_block_hash_source(&root);
        assert_ne!(with_override, edited);
        assert!(edited.contains("edited plan"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn commands_block_hash_source_changes_when_a_target_playbook_overlay_path_or_content_changes() {
        let root = tmp_dir("hash-source-target-overlay");
        write_config(
            &root,
            "targets:\n  api:\n    match: [apps/api/**]\n    playbooks: { test: overlay.md }\n",
        );
        fs::write(root.join("overlay.md"), "v1\n").unwrap();
        let v1 = commands_block_hash_source(&root);
        assert!(v1.contains("overlay.md"));
        assert!(v1.contains("v1"));

        fs::write(root.join("overlay.md"), "v2\n").unwrap();
        let v2 = commands_block_hash_source(&root);
        assert_ne!(v1, v2);
        assert!(v2.contains("v2"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn legacy_hash_source_never_includes_playbooks_even_when_present() {
        let root = tmp_dir("hash-source-legacy-ignores-playbooks");
        write_config(
            &root,
            "commands:\n  test: echo hi\ntargets:\n  api:\n    match: [apps/api/**]\n    playbooks: { test: overlay.md }\n",
        );
        fs::write(root.join("overlay.md"), "overlay\n").unwrap();
        fs::create_dir_all(root.join(".gate/playbooks")).unwrap();
        fs::write(root.join(".gate/playbooks/plan.md"), "override\n").unwrap();

        let legacy = commands_block_hash_source_legacy(&root);
        assert!(!legacy.contains("playbook"));
        assert!(!legacy.contains("overlay"));
        assert!(!legacy.contains("override"));
        assert_eq!(
            legacy,
            "{\"commands\":{\"test\":\"echo hi\"},\"targets\":{\"api\":{\"match\":[\"apps/api/**\"],\"commands\":{}}}}"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn legacy_hash_source_matches_current_source_when_no_playbook_coverage_exists() {
        let root = tmp_dir("hash-source-legacy-matches-current");
        write_config(
            &root,
            "commands:\n  test: echo hi\nscope_ignore:\n  - dist/**\n",
        );
        assert_eq!(
            commands_block_hash_source_legacy(&root),
            commands_block_hash_source(&root)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn trusted_playbook_paths_lists_both_override_and_overlay_sources() {
        let root = tmp_dir("trusted-playbook-paths");
        write_config(
            &root,
            "targets:\n  api:\n    match: [apps/api/**]\n    playbooks: { test: overlay.md }\n",
        );
        fs::write(root.join("overlay.md"), "overlay\n").unwrap();
        fs::create_dir_all(root.join(".gate/playbooks")).unwrap();
        fs::write(root.join(".gate/playbooks/plan.md"), "override\n").unwrap();

        let mut paths = trusted_playbook_paths(&root);
        paths.sort();
        assert_eq!(
            paths,
            vec![
                ".gate/playbooks/plan.md".to_string(),
                "overlay.md".to_string()
            ]
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
