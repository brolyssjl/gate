//! Port of `src/gates/coverage.ts` (`loadCoverage`, `FileCoverage`,
//! `DiffCoverage`, format-specific parsers for istanbul/generic/coverage-py/
//! go-cover/lcov). Ported in wave 3 (see `docs/rust-port.md`).
//!
//! Milestone 1 shipped two parsers (jest/vitest + generic JSON contract);
//! Milestone 4 added three more built-in formats so non-Node stacks don't
//! fall back to hand-rolling the generic contract:
//!   - istanbul `coverage/coverage-final.json` (jest, vitest --coverage)
//!   - generic contract: `{ files: [{ path, covered:[], uncovered:[] }] }`
//!   - coverage.py `coverage/coverage.json` (`coverage json -o coverage/coverage.json`)
//!   - Go cover profile `coverage/go-cover.out` (`go test -coverprofile=coverage/go-cover.out`)
//!   - lcov `coverage/lcov.info` (nyc, gcov, and many others' standard output path)
//!
//! Each path is a Gate convention (everything lives under `coverage/`), not
//! something the underlying tool assumes - point the configured coverage
//! command at it. Returns `None` if no report is found or it can't be
//! parsed; a coverage threshold with no parseable report fails closed (see
//! the TEST gate).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::core::config::CoverageFormat;
use crate::core::json::{self, Value};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileCoverage {
    pub covered: HashSet<i64>,
    pub uncovered: HashSet<i64>,
}

/// relative POSIX path -> line coverage.
pub type CoverageMap = HashMap<String, FileCoverage>;

struct FormatSpec {
    /// Path relative to the repo root this format's report is expected at
    /// (a Gate convention, not something the underlying tool assumes).
    path: &'static str,
    parse: fn(&str, &Path) -> Option<CoverageMap>,
}

/// One registry entry per format Gate knows, in auto-detection order -
/// mirrors TS's `FORMATS` record (`Object.keys`/`Object.values` iterate in
/// insertion order for string keys), driving both auto-detection order and
/// (via `coverage_report_paths`) the "no report found" failure message.
const FORMATS: &[(CoverageFormat, FormatSpec)] = &[
    (
        CoverageFormat::Generic,
        FormatSpec {
            path: "coverage/gate-coverage.json",
            parse: parse_generic,
        },
    ),
    (
        CoverageFormat::Istanbul,
        FormatSpec {
            path: "coverage/coverage-final.json",
            parse: parse_istanbul,
        },
    ),
    (
        CoverageFormat::CoveragePy,
        FormatSpec {
            path: "coverage/coverage.json",
            parse: parse_coverage_py,
        },
    ),
    (
        CoverageFormat::GoCover,
        FormatSpec {
            path: "coverage/go-cover.out",
            parse: parse_go_cover,
        },
    ),
    (
        CoverageFormat::Lcov,
        FormatSpec {
            path: "coverage/lcov.info",
            parse: parse_lcov,
        },
    ),
];

/// Every format's expected report path, for the "no coverage report was
/// found" failure message.
pub fn coverage_report_paths() -> Vec<&'static str> {
    FORMATS.iter().map(|(_, spec)| spec.path).collect()
}

pub fn load_coverage(root: &Path, format: CoverageFormat) -> Option<CoverageMap> {
    for (name, spec) in FORMATS {
        if format != CoverageFormat::Auto && format != *name {
            continue;
        }
        let full = root.join(spec.path);
        let Ok(raw) = fs::read_to_string(&full) else {
            continue;
        };
        if let Some(parsed) = (spec.parse)(&raw, root) {
            return Some(parsed);
        }
        // Exists but didn't parse: fall through and keep trying other
        // formats in "auto" mode (an explicit single format has nothing
        // left to fall back to).
    }
    None
}

fn to_rel(root: &Path, p: &str) -> String {
    // A leading "./" (some lcov writers - genhtml, certain nyc/cargo-llvm-cov
    // configs - emit `SF:./src/file.ts`) is normalized away, not just the
    // absolute-path case every other parser already handles.
    let path = Path::new(p);
    let rel = if path.is_absolute() {
        pathdiff(path, root)
    } else {
        normalize_dot_segments(p)
    };
    rel.replace('\\', "/")
}

/// `path.relative(root, p)` for the absolute-path case: since Gate targets
/// darwin/linux, both `root` and `p` share the same absolute prefix once
/// resolved - strip `root` off `p`'s components.
fn pathdiff(p: &Path, root: &Path) -> String {
    match p.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().to_string(),
        Err(_) => p.to_string_lossy().to_string(),
    }
}

/// `node:path`'s `normalize()` for a relative path: collapses `./`, `//`,
/// and (harmlessly, since these paths never legitimately need to escape the
/// repo) leaves the string as-is if it doesn't start with `./`.
fn normalize_dot_segments(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "." | "" => continue,
            _ => out.push(seg),
        }
    }
    if out.is_empty() {
        ".".to_string()
    } else {
        out.join("/")
    }
}

/// Record one line's hit/miss into a file's coverage entry. A covered hit is
/// never overwritten by a later uncovered record of the same line (an
/// earlier uncovered mark yields to a later covered one instead - see
/// `reconcile_coverage`), so callers don't need to check
/// `covered.contains(line)` themselves before adding to `uncovered`.
fn mark_line(entry: &mut FileCoverage, line: i64, hit: bool) {
    if hit {
        entry.covered.insert(line);
    } else if !entry.covered.contains(&line) {
        entry.uncovered.insert(line);
    }
}

/// Final reconcile pass, shared by every parser that can see the same line
/// more than once under different verdicts (istanbul: sibling statements on
/// one line; go-cover: overlapping blocks from a merged multi-package
/// profile; lcov: separate test-suite records for one SF). A covered hit
/// anywhere always wins over an uncovered record of the same line recorded
/// earlier, regardless of order.
fn reconcile_coverage(map: &mut CoverageMap) {
    for entry in map.values_mut() {
        for line in &entry.covered {
            entry.uncovered.remove(line);
        }
    }
}

fn as_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Int(n) => Some(*n),
        Value::Float(f) => Some(*f as i64),
        _ => None,
    }
}

fn as_i64_array(v: Option<&Value>) -> HashSet<i64> {
    match v {
        Some(Value::Array(items)) => items.iter().filter_map(as_i64).collect(),
        _ => HashSet::new(),
    }
}

fn parse_istanbul(raw: &str, root: &Path) -> Option<CoverageMap> {
    let data = json::parse(raw).ok()?;
    let entries = data.as_object()?;
    let mut map = CoverageMap::new();
    for (key, file) in entries {
        let Some(statement_map) = file.get("statementMap").and_then(|v| v.as_object()) else {
            continue;
        };
        let Some(s) = file.get("s") else { continue };
        let path = file
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(key.as_str());
        let rel = to_rel(root, path);
        let mut entry = FileCoverage::default();
        for (id, stmt) in statement_map {
            let start = stmt
                .get("start")
                .and_then(|v| v.get("line"))
                .and_then(as_i64);
            let end = stmt.get("end").and_then(|v| v.get("line")).and_then(as_i64);
            let (Some(start), Some(end)) = (start, end) else {
                continue;
            };
            let hits = s.get(id).and_then(as_i64).unwrap_or(0);
            let mut line = start;
            while line <= end {
                mark_line(&mut entry, line, hits > 0);
                line += 1;
            }
        }
        map.insert(rel, entry);
    }
    reconcile_coverage(&mut map);
    Some(map)
}

fn parse_generic(raw: &str, root: &Path) -> Option<CoverageMap> {
    let data = json::parse(raw).ok()?;
    let Some(Value::Array(files)) = data.get("files") else {
        return None;
    };
    let mut map = CoverageMap::new();
    for f in files {
        let Some(path) = f.get("path").and_then(|v| v.as_str()) else {
            continue;
        };
        map.insert(
            to_rel(root, path),
            FileCoverage {
                covered: as_i64_array(f.get("covered")),
                uncovered: as_i64_array(f.get("uncovered")),
            },
        );
    }
    Some(map)
}

/// coverage.py's `coverage json` output: `{ files: { <path>: {
/// executed_lines, missing_lines } } }`.
fn parse_coverage_py(raw: &str, root: &Path) -> Option<CoverageMap> {
    let data = json::parse(raw).ok()?;
    let files = data.get("files").and_then(|v| v.as_object())?;
    let mut map = CoverageMap::new();
    for (path, file) in files {
        if file.as_object().is_none() {
            continue;
        }
        map.insert(
            to_rel(root, path),
            FileCoverage {
                covered: as_i64_array(file.get("executed_lines")),
                uncovered: as_i64_array(file.get("missing_lines")),
            },
        );
    }
    Some(map)
}

/// `module <name>` line of go.mod, with a trailing slash, so profile paths
/// can be de-prefixed to repo-relative. `None` when there's no go.mod to
/// read.
fn read_go_module_prefix(root: &Path) -> Option<String> {
    let gomod = fs::read_to_string(root.join("go.mod")).ok()?;
    for line in gomod.lines() {
        let Some(rest) = line.strip_prefix("module") else {
            continue;
        };
        if !rest.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        if let Some(name) = rest.split_whitespace().next() {
            return Some(format!("{name}/"));
        }
    }
    None
}

fn parse_line_dot_col(s: &str) -> Option<i64> {
    let (line_str, col_str) = s.split_once('.')?;
    if line_str.is_empty() || !line_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if col_str.is_empty() || !col_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    line_str.parse().ok()
}

/// One `go test -coverprofile` block line: `<file>:<startLine>.<col>,
/// <endLine>.<col> <numStatements> <count>`. Mirrors
/// `^(.+):(\d+)\.\d+,(\d+)\.\d+\s+\d+\s+(\d+)$` - `numStatements` is matched
/// but never captured/used, same as the TS destructure.
fn parse_go_cover_block(line: &str) -> Option<(String, i64, i64, i64)> {
    let colon = line.rfind(':')?;
    let file = &line[..colon];
    if file.is_empty() {
        return None;
    }
    let rest = &line[colon + 1..];
    let mut parts = rest.split_whitespace();
    let coords = parts.next()?;
    let num_statements = parts.next()?;
    let count = parts.next()?;
    if parts.next().is_some() {
        return None; // trailing junk - `$` anchor in the TS regex rejects this
    }
    if num_statements.is_empty() || !num_statements.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (start_part, end_part) = coords.split_once(',')?;
    let start = parse_line_dot_col(start_part)?;
    let end = parse_line_dot_col(end_part)?;
    let count: i64 = count.parse().ok()?;
    Some((file.to_string(), start, end, count))
}

/// `go test -coverprofile` text profile: a `mode: <set|count|atomic>`
/// header, then one line per code block. File paths are Go import paths
/// (`<module>/<repo-relative path>`), not filesystem paths - stripped
/// against go.mod's module name so they line up with git's repo-relative
/// paths; without a go.mod (or a module path that doesn't match) they're
/// kept as-is, which degrades to "no coverage data for this file" rather
/// than crashing (the same fate as any file `diff_coverage` doesn't
/// recognize).
fn parse_go_cover(raw: &str, root: &Path) -> Option<CoverageMap> {
    let lines: Vec<&str> = raw
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let first = lines.first()?;
    if !(first.starts_with("mode:") && first["mode:".len()..].split_whitespace().count() == 1) {
        return None;
    }

    let prefix = read_go_module_prefix(root);
    let mut map = CoverageMap::new();
    for line in &lines[1..] {
        let Some((file, start, end, count)) = parse_go_cover_block(line) else {
            continue; // tolerate a stray/blank line rather than failing the whole report
        };
        let rel = match &prefix {
            Some(p) if file.starts_with(p.as_str()) => file[p.len()..].to_string(),
            _ => file,
        };
        let entry = map.entry(rel).or_default();
        let hit = count > 0;
        let mut ln = start;
        while ln <= end {
            mark_line(entry, ln, hit);
            ln += 1;
        }
    }
    reconcile_coverage(&mut map);
    Some(map)
}

/// Standard lcov `.info` text format: `SF:<path>` starts a file record,
/// `DA:<line>,<hits>` reports one line's hit count, `end_of_record` closes
/// it. Multiple records for the same `SF` (e.g. separate test suites)
/// accumulate into the same file entry. Function/branch lines
/// (FN/FNDA/BRDA) are ignored - Gate measures line coverage only. `SF:`
/// paths are relativized via `to_rel` like every other parser.
fn parse_lcov(raw: &str, root: &Path) -> Option<CoverageMap> {
    if !raw.lines().any(|l| l.trim().starts_with("SF:")) {
        return None;
    }
    let mut map = CoverageMap::new();
    let mut current: Option<String> = None;
    for raw_line in raw.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("SF:") {
            let path = to_rel(root, rest.trim());
            map.entry(path.clone()).or_default();
            current = Some(path);
        } else if let Some(rest) = line.strip_prefix("DA:") {
            let Some(cur) = &current else { continue };
            let Some((line_str, hits_str)) = rest.split_once(',') else {
                continue;
            };
            let (Ok(ln), Ok(hits)) = (line_str.parse::<i64>(), hits_str.parse::<i64>()) else {
                continue;
            };
            let entry = map.get_mut(cur).unwrap();
            mark_line(entry, ln, hits > 0);
        } else if line == "end_of_record" {
            current = None;
        }
    }
    reconcile_coverage(&mut map);
    Some(map)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoverageGap {
    pub file: String,
    pub uncovered: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffCoverage {
    pub changed_executable: i64,
    pub covered: i64,
    /// 0-100; 100 when there are no measurable changed lines.
    pub percent: i64,
    /// Files with uncovered changed lines, for the failure message.
    pub gaps: Vec<CoverageGap>,
}

/// Diff coverage: of the executable lines changed in this run, how many are
/// covered by the tests. `changed` maps file -> changed new-file line
/// numbers (untracked files arrive with every line enumerated); an empty
/// set means no added lines - e.g. a deletion-only diff - and contributes
/// nothing. Takes an ordered slice (mirrors `core::git::changed_lines`'s
/// `Vec`, matching JS `Map` iteration/insertion order) so the gap list's
/// order - and therefore the failure message's first-5-gaps slice - is
/// deterministic.
pub fn diff_coverage(changed: &[(String, HashSet<i64>)], coverage: &CoverageMap) -> DiffCoverage {
    let mut changed_executable = 0i64;
    let mut covered = 0i64;
    let mut gaps: Vec<CoverageGap> = Vec::new();

    for (file, changed_set) in changed {
        let Some(cov) = coverage.get(file) else {
            continue; // no coverage data for this file (e.g. non-source) - skip
        };
        let executable: HashSet<i64> = cov.covered.union(&cov.uncovered).copied().collect();
        let mut target: Vec<i64> = changed_set
            .iter()
            .filter(|l| executable.contains(l))
            .copied()
            .collect();
        target.sort_unstable();
        let mut missed: Vec<i64> = Vec::new();
        for line in target {
            changed_executable += 1;
            if cov.covered.contains(&line) {
                covered += 1;
            } else {
                missed.push(line);
            }
        }
        if !missed.is_empty() {
            missed.sort_unstable();
            gaps.push(CoverageGap {
                file: file.clone(),
                uncovered: missed,
            });
        }
    }

    let percent = if changed_executable == 0 {
        100
    } else {
        (covered * 100) / changed_executable
    };
    DiffCoverage {
        changed_executable,
        covered,
        percent,
        gaps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        crate::core::testutil::unique_temp_dir(&format!("coverage-rs-{name}"))
    }

    fn with_coverage_file(name: &str, filename: &str, content: &str) -> std::path::PathBuf {
        let root = tmp_dir(name);
        stdfs::create_dir_all(root.join("coverage")).unwrap();
        stdfs::write(root.join("coverage").join(filename), content).unwrap();
        root
    }

    fn set(items: &[i64]) -> HashSet<i64> {
        items.iter().copied().collect()
    }

    fn sample_coverage() -> CoverageMap {
        let mut m = CoverageMap::new();
        m.insert(
            "src/a.ts".to_string(),
            FileCoverage {
                covered: set(&[1, 2, 3]),
                uncovered: set(&[4, 5]),
            },
        );
        m
    }

    #[test]
    fn diff_coverage_is_100_when_covered_changed_lines_are_all_covered() {
        let changed = vec![("src/a.ts".to_string(), set(&[1, 2]))];
        assert_eq!(diff_coverage(&changed, &sample_coverage()).percent, 100);
    }

    #[test]
    fn diff_coverage_computes_the_covered_fraction_of_changed_executable_lines() {
        let changed = vec![("src/a.ts".to_string(), set(&[1, 2, 4]))];
        let dc = diff_coverage(&changed, &sample_coverage());
        assert_eq!(dc.changed_executable, 3);
        assert_eq!(dc.covered, 2);
        assert_eq!(dc.percent, 66);
        assert_eq!(
            dc.gaps,
            vec![CoverageGap {
                file: "src/a.ts".to_string(),
                uncovered: vec![4]
            }]
        );
    }

    #[test]
    fn diff_coverage_ignores_changed_lines_with_no_coverage_data() {
        let changed = vec![("README.md".to_string(), set(&[1, 2]))];
        assert_eq!(diff_coverage(&changed, &sample_coverage()).percent, 100);
    }

    #[test]
    fn diff_coverage_treats_an_empty_changed_set_as_nothing_to_measure() {
        let changed = vec![("src/a.ts".to_string(), HashSet::new())];
        let dc = diff_coverage(&changed, &sample_coverage());
        assert_eq!(dc.changed_executable, 0);
        assert_eq!(dc.percent, 100);
        assert_eq!(dc.gaps, vec![]);
    }

    #[test]
    fn diff_coverage_reports_100_when_nothing_measurable_changed() {
        assert_eq!(diff_coverage(&[], &sample_coverage()).percent, 100);
    }

    #[test]
    fn load_coverage_coverage_py_parses_executed_and_missing_lines() {
        let report = r#"{"meta":{"format":2},"files":{"pkg/mod.py":{"executed_lines":[1,2,3],"missing_lines":[4,5]}},"totals":{}}"#;
        let root = with_coverage_file("py-parse", "coverage.json", report);
        let map = load_coverage(&root, CoverageFormat::CoveragePy).unwrap();
        assert_eq!(
            map.get("pkg/mod.py").unwrap(),
            &FileCoverage {
                covered: set(&[1, 2, 3]),
                uncovered: set(&[4, 5])
            }
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn load_coverage_auto_detects_coverage_py() {
        let report = r#"{"files":{"pkg/mod.py":{"executed_lines":[1],"missing_lines":[]}}}"#;
        let root = with_coverage_file("py-auto", "coverage.json", report);
        let map = load_coverage(&root, CoverageFormat::Auto).unwrap();
        assert_eq!(map.get("pkg/mod.py").unwrap().covered, set(&[1]));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn load_coverage_fails_closed_on_unparseable_json() {
        let root = with_coverage_file("py-badjson", "coverage.json", "not json at all");
        assert!(load_coverage(&root, CoverageFormat::CoveragePy).is_none());
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn load_coverage_fails_closed_when_the_report_has_no_files_key() {
        let root = with_coverage_file("py-nofiles", "coverage.json", r#"{"totals":{}}"#);
        assert!(load_coverage(&root, CoverageFormat::CoveragePy).is_none());
        stdfs::remove_dir_all(&root).unwrap();
    }

    fn go_profile() -> String {
        [
            "mode: set",
            "example.com/mod/pkg/file.go:1.1,3.2 2 1",
            "example.com/mod/pkg/file.go:4.1,4.10 1 0",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn go_cover_expands_each_blocks_line_range_stripping_the_module_prefix() {
        let root = with_coverage_file("go-strip", "go-cover.out", &go_profile());
        stdfs::write(root.join("go.mod"), "module example.com/mod\n\ngo 1.22\n").unwrap();
        let map = load_coverage(&root, CoverageFormat::GoCover).unwrap();
        assert_eq!(
            map.get("pkg/file.go").unwrap(),
            &FileCoverage {
                covered: set(&[1, 2, 3]),
                uncovered: set(&[4])
            }
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn go_cover_keeps_the_raw_profile_path_with_no_go_mod() {
        let root = with_coverage_file("go-nomod", "go-cover.out", &go_profile());
        let map = load_coverage(&root, CoverageFormat::GoCover).unwrap();
        assert!(map.contains_key("example.com/mod/pkg/file.go"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn go_cover_fails_closed_on_a_file_with_no_mode_header() {
        let root = with_coverage_file(
            "go-nomode",
            "go-cover.out",
            "not a go cover profile\nrandom text\n",
        );
        assert!(load_coverage(&root, CoverageFormat::GoCover).is_none());
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn go_cover_fails_closed_on_an_empty_file() {
        let root = with_coverage_file("go-empty", "go-cover.out", "");
        assert!(load_coverage(&root, CoverageFormat::GoCover).is_none());
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn go_cover_a_later_blocks_covered_hit_wins_over_an_earlier_uncovered_one() {
        let overlapping = [
            "mode: set",
            "example.com/mod/pkg/file.go:1.1,2.2 1 0",
            "example.com/mod/pkg/file.go:2.1,3.2 1 1",
            "",
        ]
        .join("\n");
        let root = with_coverage_file("go-overlap", "go-cover.out", &overlapping);
        let map = load_coverage(&root, CoverageFormat::GoCover).unwrap();
        assert_eq!(
            map.get("example.com/mod/pkg/file.go").unwrap(),
            &FileCoverage {
                covered: set(&[2, 3]),
                uncovered: set(&[1])
            }
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    fn lcov_sample() -> String {
        [
            "SF:src/file.ts",
            "DA:1,1",
            "DA:2,0",
            "DA:3,1",
            "end_of_record",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn lcov_parses_da_lines_into_covered_uncovered_per_sf_block() {
        let root = with_coverage_file("lcov-basic", "lcov.info", &lcov_sample());
        let map = load_coverage(&root, CoverageFormat::Lcov).unwrap();
        assert_eq!(
            map.get("src/file.ts").unwrap(),
            &FileCoverage {
                covered: set(&[1, 3]),
                uncovered: set(&[2])
            }
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lcov_relativizes_an_absolute_sf_path() {
        let root = tmp_dir("lcov-absolute");
        stdfs::create_dir_all(root.join("coverage")).unwrap();
        let absolute = root.join("src").join("file.ts");
        stdfs::write(
            root.join("coverage").join("lcov.info"),
            format!("SF:{}\nDA:1,1\nDA:2,0\nend_of_record\n", absolute.display()),
        )
        .unwrap();
        let map = load_coverage(&root, CoverageFormat::Lcov).unwrap();
        assert_eq!(
            map.get("src/file.ts").unwrap(),
            &FileCoverage {
                covered: set(&[1]),
                uncovered: set(&[2])
            }
        );
        assert!(!map.contains_key(absolute.to_str().unwrap()));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lcov_relativizes_a_dot_slash_prefixed_sf_path() {
        let root = with_coverage_file(
            "lcov-dotslash",
            "lcov.info",
            &["SF:./src/file.ts", "DA:1,1", "DA:2,0", "end_of_record", ""].join("\n"),
        );
        let map = load_coverage(&root, CoverageFormat::Lcov).unwrap();
        assert_eq!(
            map.get("src/file.ts").unwrap(),
            &FileCoverage {
                covered: set(&[1]),
                uncovered: set(&[2])
            }
        );
        assert!(!map.contains_key("./src/file.ts"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lcov_merges_multiple_record_blocks_for_the_same_file() {
        let two_blocks = [
            "SF:src/file.ts",
            "DA:1,0",
            "end_of_record",
            "SF:src/file.ts",
            "DA:1,1",
            "DA:2,1",
            "end_of_record",
            "",
        ]
        .join("\n");
        let root = with_coverage_file("lcov-merge", "lcov.info", &two_blocks);
        let map = load_coverage(&root, CoverageFormat::Lcov).unwrap();
        assert_eq!(
            map.get("src/file.ts").unwrap(),
            &FileCoverage {
                covered: set(&[1, 2]),
                uncovered: HashSet::new()
            }
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lcov_fails_closed_with_no_sf_record_at_all() {
        let root = with_coverage_file("lcov-none", "lcov.info", "not an lcov file\njust text\n");
        assert!(load_coverage(&root, CoverageFormat::Lcov).is_none());
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn istanbul_marks_every_line_in_a_multi_line_statement() {
        let report = r#"{"/repo/src/a.js":{"path":"src/a.js","statementMap":{"0":{"start":{"line":1},"end":{"line":3}}},"s":{"0":1}}}"#;
        let root = with_coverage_file("istanbul-basic", "coverage-final.json", report);
        // istanbul lives at coverage/coverage-final.json per the registry.
        let map = load_coverage(&root, CoverageFormat::Istanbul).unwrap();
        assert_eq!(
            map.get("src/a.js").unwrap(),
            &FileCoverage {
                covered: set(&[1, 2, 3]),
                uncovered: HashSet::new()
            }
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn generic_contract_reads_covered_and_uncovered_arrays_directly() {
        let report = r#"{"files":[{"path":"src/a.ts","covered":[1,2],"uncovered":[3]}]}"#;
        let root = with_coverage_file("generic-basic", "gate-coverage.json", report);
        let map = load_coverage(&root, CoverageFormat::Generic).unwrap();
        assert_eq!(
            map.get("src/a.ts").unwrap(),
            &FileCoverage {
                covered: set(&[1, 2]),
                uncovered: set(&[3])
            }
        );
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn coverage_report_paths_lists_every_formats_expected_path() {
        let paths = coverage_report_paths();
        assert!(paths.contains(&"coverage/coverage-final.json"));
        assert!(paths.contains(&"coverage/gate-coverage.json"));
        assert!(paths.contains(&"coverage/coverage.json"));
        assert!(paths.contains(&"coverage/go-cover.out"));
        assert!(paths.contains(&"coverage/lcov.info"));
    }
}
