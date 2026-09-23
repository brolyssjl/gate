//! Port of `src/integrations/agnosgramWrite.ts` (`formatJournalEntry`,
//! journal write via the `agnosgram` CLI with a fallback). Ported in wave 3
//! (see `docs/rust-port.md`), via `std::process::Command`.
//!
//! Writes a `gate retro` journal entry into `.agnosgram/journal/`, in the
//! frozen Agnosgram journal format v1 (pinned cross-project contract):
//!
//! ```text
//! ## YYYY-MM-DD HH:MM · gate · <branch>
//! - **Did:** Completed gate run "<title>" (<run-id>, profile <profile>)
//! - **Learned:** <broke entries, joined " · ">
//! - **Decided:** <conventions entries>
//! - **Avoid:** <avoid entries>
//! - **Source:** .gate/runs/<run-id>
//! ```
//!
//! Empty slots are omitted. Multiple entries in one slot stay on that
//! slot's single line (the pinned v1 contract is one line per slot, and
//! this repo has no way to confirm agnosgram's own parser tolerates a
//! nested bullet list under a slot heading) - joined with " · " rather
//! than "; ", both to read more cleanly and to avoid colliding with a
//! semicolon that might legitimately appear inside one entry's own prose.
//! `format_journal_entry` is pure and golden-tested against that exact
//! block; everything else here is I/O.
//!
//! No TS counterpart: local-time formatting via `localtime_r` (direct
//! `extern "C"` binding), the same pattern agnosgram's Rust port uses for
//! `src/commands/log.ts`'s journal timestamps - see `docs/rust-port.md`'s
//! dependency policy, which calls this pattern out by name.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::artifacts::retro::RetroLog;
use crate::cli::output::UserError;
use crate::core::fsx::write_file_atomic;
use crate::core::run::civil_from_days;

/// TS takes `run: { id, title, profile }` (a duck-typed slice of `Run`), not
/// the full `Run` - mirrored here as borrowed fields rather than introducing
/// a struct with no TS counterpart.
pub struct JournalEntryParams<'a> {
    pub run_id: &'a str,
    pub run_title: &'a str,
    pub run_profile: &'a str,
    pub retro: &'a RetroLog,
    pub branch: Option<&'a str>,
    /// Unix epoch seconds; `None` defaults to now (mirrors TS's `when: Date
    /// = new Date()` default parameter).
    pub when: Option<i64>,
    /// No TS counterpart. `None` (every real caller, e.g. `gate retro`) uses
    /// the process's ambient local time zone, same as always. `Some(offset)`
    /// pins the heading's timestamp to that fixed UTC offset (seconds)
    /// instead, bypassing `local_timestamp_at`'s `tzset()`/`localtime_r`
    /// call entirely - this is what lets this module's tests fix "UTC"
    /// deterministically without mutating the process-global `TZ` env var,
    /// which used to race other threads under `cargo test`'s parallel
    /// runner (see `local_timestamp_with_offset`).
    pub tz_offset_secs: Option<i64>,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    // macOS and glibc both carry these two extra fields with this same
    // leading layout; we only target those two platforms (see
    // `docs/rust-port.md`).
    tm_gmtoff: i64,
    tm_zone: *const std::os::raw::c_char,
}

extern "C" {
    fn localtime_r(timer: *const i64, result: *mut Tm) -> *mut Tm;
    fn tzset();
}

/// `YYYY-MM-DD HH:MM` for `epoch_secs` shifted by a fixed UTC offset
/// (seconds, positive east of UTC) - pure civil-calendar arithmetic, no FFI,
/// no env. This is the same identity `localtime_r` itself relies on (local
/// broken-down time is UTC shifted by the zone's offset at that instant,
/// modulo the leap seconds neither side accounts for), so it doubles as
/// both the UTC fallback below and the deterministic path tests use to pin
/// an offset without touching the process-global `TZ` env var (see
/// `JournalEntryParams::tz_offset_secs`).
fn local_timestamp_with_offset(epoch_secs: i64, gmtoff_secs: i64) -> String {
    let local_secs = epoch_secs + gmtoff_secs;
    let (y, m, d) = civil_from_days(local_secs.div_euclid(86_400));
    let rem = local_secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// `YYYY-MM-DD HH:MM` in local time for a given Unix instant, exactly as
/// `src/integrations/agnosgramWrite.ts`'s `localTimestamp` renders a `Date`.
/// Calls `tzset()` first so a `TZ` env change since the last call is picked
/// up (matters for callers that need a specific zone; a no-op in the common
/// case where `TZ` never changes during the process's lifetime).
fn local_timestamp_at(epoch_secs: i64) -> String {
    unsafe { tzset() };
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !localtime_r(&epoch_secs, &mut tm).is_null() };
    let gmtoff = if ok {
        tm.tm_gmtoff
    } else {
        // Fall back to UTC rather than panicking - this should not happen on
        // darwin/linux, but a journal entry is more useful with a UTC
        // timestamp than with a crashed `gate retro`.
        0
    };
    local_timestamp_with_offset(epoch_secs, gmtoff)
}

pub fn format_journal_entry(params: &JournalEntryParams) -> String {
    let when = params.when.unwrap_or_else(now_secs);

    let timestamp = match params.tz_offset_secs {
        Some(offset) => local_timestamp_with_offset(when, offset),
        None => local_timestamp_at(when),
    };
    let mut heading_parts = vec![
        format!("## {timestamp}"),
        "·".to_string(),
        "gate".to_string(),
    ];
    if let Some(branch) = params.branch {
        heading_parts.push("·".to_string());
        heading_parts.push(branch.to_string());
    }

    let mut lines = vec![heading_parts.join(" ")];
    lines.push(format!(
        "- **Did:** Completed gate run \"{}\" ({}, profile {})",
        params.run_title, params.run_id, params.run_profile
    ));
    if !params.retro.broke.is_empty() {
        lines.push(format!("- **Learned:** {}", params.retro.broke.join(" · ")));
    }
    if !params.retro.conventions.is_empty() {
        lines.push(format!(
            "- **Decided:** {}",
            params.retro.conventions.join(" · ")
        ));
    }
    if !params.retro.avoid.is_empty() {
        lines.push(format!("- **Avoid:** {}", params.retro.avoid.join(" · ")));
    }
    lines.push(format!("- **Source:** .gate/runs/{}", params.run_id));

    lines.join("\n") + "\n"
}

/// Fixed argv, binary resolved once per call from `$GATE_AGNOSGRAM_BIN` (or
/// `agnosgram` on PATH by default) - no arguments read from repo config, so
/// this sits outside Gate's command-trust regime (TOFU): there is nothing a
/// hostile `.gate/config.yml` could inject, because config never reaches
/// this call. The env override exists for test hermeticity: whether a real
/// `agnosgram` happens to be installed on the machine running the suite
/// must not change what these tests exercise.
fn agnosgram_bin() -> String {
    env::var("GATE_AGNOSGRAM_BIN").unwrap_or_else(|_| "agnosgram".to_string())
}
const AGNOSGRAM_ARGS: [&str; 4] = ["log", "--stdin", "--agent", "gate"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalWriteMethod {
    AgnosgramCli,
    Fallback,
}

impl JournalWriteMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            JournalWriteMethod::AgnosgramCli => "agnosgram-cli",
            JournalWriteMethod::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalWriteResult {
    /// Repo-relative POSIX path to the journal file the entry landed in.
    pub journal_file: String,
    pub method: JournalWriteMethod,
}

/// The journal month, in UTC - matching the Agnosgram CLI's own contract
/// (`toISOString().slice(0, 7)`). Using local time here would pick a
/// different month than the CLI near a month boundary in any timezone ahead
/// of or behind UTC, producing wrong receipts (the file Gate records in
/// run.json isn't the one the CLI actually wrote to) and duplicate entries
/// on retry. Must stay in sync in both the CLI-success path and the ENOENT
/// direct-append fallback below - both funnel through this one function.
fn journal_month_utc(epoch_secs: i64) -> String {
    let (y, m, _) = civil_from_days(epoch_secs.div_euclid(86_400));
    format!("{y:04}-{m:02}")
}

/// Repo-relative POSIX path to the month's journal file `epoch_secs` falls
/// in (UTC).
pub fn journal_file_path(epoch_secs: i64) -> String {
    format!(".agnosgram/journal/{}.md", journal_month_utc(epoch_secs))
}

fn month_header(month: &str) -> String {
    format!(
        "# Journal - {month}\n\nAppend-only. One file per month. Written by `agnosgram log` and, when the\nCLI is unavailable, by `gate retro`'s direct-append fallback.\n"
    )
}

/// Append `entry` to the current month's journal file, creating the file
/// (and `.agnosgram/journal/`) if this is its first entry. Used only when
/// spawning the agnosgram CLI fails with ENOENT.
fn append_journal_direct(root: &Path, entry: &str, epoch_secs: i64) -> io::Result<String> {
    let rel_path = journal_file_path(epoch_secs);
    let abs_path = root.join(&rel_path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = if abs_path.exists() {
        fs::read_to_string(&abs_path)?
    } else {
        month_header(&journal_month_utc(epoch_secs))
    };
    let with_trailing_newline = if existing.ends_with('\n') {
        existing
    } else {
        existing + "\n"
    };
    write_file_atomic(&abs_path, &(with_trailing_newline + "\n" + entry))?;
    Ok(rel_path)
}

/// Write a pre-formatted journal entry: prefer spawning the agnosgram CLI
/// (`agnosgram log --stdin --agent gate`, entry piped on stdin - agnosgram
/// appends stdin verbatim when it already starts with `##`); when the
/// binary isn't installed (ENOENT), fall back to appending directly in the
/// same frozen format. Any other spawn failure (binary present but errored)
/// is surfaced to the caller rather than silently falling back, since that
/// usually means something is actually wrong with the store.
pub fn write_journal_entry(
    root: &Path,
    entry: &str,
    when: Option<i64>,
) -> Result<JournalWriteResult, UserError> {
    write_journal_entry_with(&agnosgram_bin(), root, entry, when)
}

/// [`write_journal_entry`] with the agnosgram binary passed in explicitly
/// instead of resolved from `$GATE_AGNOSGRAM_BIN`. This is the seam the unit
/// tests use to point at a stub, or at a path that does not exist to force
/// the ENOENT fallback, without mutating process-global environment - which
/// would race every other test in the same `cargo test` process (2026-09-22
/// audit finding 11).
fn write_journal_entry_with(
    bin: &str,
    root: &Path,
    entry: &str,
    when: Option<i64>,
) -> Result<JournalWriteResult, UserError> {
    let epoch = when.unwrap_or_else(now_secs);

    let mut child = match Command::new(bin)
        .args(AGNOSGRAM_ARGS)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let journal_file = append_journal_direct(root, entry, epoch)
                .map_err(|e| UserError::new(e.to_string()))?;
            return Ok(JournalWriteResult {
                journal_file,
                method: JournalWriteMethod::Fallback,
            });
        }
        Err(e) => return Err(UserError::new(format!("agnosgram log failed: {e}"))),
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(entry.as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|e| UserError::new(format!("agnosgram log failed: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        return Err(UserError::new(format!("agnosgram log failed: {detail}")));
    }

    Ok(JournalWriteResult {
        journal_file: journal_file_path(epoch),
        method: JournalWriteMethod::AgnosgramCli,
    })
}

/// Whether the journal file at `journal_file` (repo-relative) already
/// records `run_id`.
pub fn journal_contains_run_id(root: &Path, journal_file: &str, run_id: &str) -> bool {
    let path = root.join(journal_file);
    match fs::read_to_string(&path) {
        Ok(content) => content.contains(run_id),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;
    use std::path::PathBuf;

    // 2026-07-27 14:05 UTC. Tests that care about the rendered heading pass
    // `tz_offset_secs: Some(0)` so this reads as the same local wall-clock
    // time the TS suite hardcodes via a local-time `Date` constructor - a
    // deliberate, documented adaptation (see this module's doc comment on
    // `local_timestamp_at`): the TS test fixture is a local `Date`, which is
    // not portably reproducible across machines/CI runners with different
    // system timezones, so this port pins the offset explicitly instead.
    // Pinning it as a parameter (rather than the process-global `TZ` env
    // var, as an earlier version of this suite did) means these tests don't
    // race other threads under `cargo test`'s parallel runner - see
    // `JournalEntryParams::tz_offset_secs`.
    const WHEN: i64 = 1785161100; // Date.UTC(2026, 6, 27, 14, 5)

    /// A binary path that cannot exist, to force the ENOENT fallback. Passed
    /// straight into `write_journal_entry_with` - earlier versions of these
    /// tests set `GATE_AGNOSGRAM_BIN` in-process behind a mutex instead,
    /// which still raced every other test reading env in the same `cargo
    /// test` process (2026-09-22 audit finding 11).
    const NO_SUCH_BIN: &str = "/nonexistent/gate-agnosgram-test-stub";

    fn tmp_repo(name: &str) -> PathBuf {
        let dir = crate::core::testutil::unique_temp_dir(&format!("agnosgramwrite-rs-{name}"));
        dir
    }

    fn empty_retro() -> RetroLog {
        RetroLog {
            broke: vec![],
            avoid: vec![],
            conventions: vec![],
            body: String::new(),
        }
    }

    #[test]
    fn renders_every_slot_when_broke_avoid_and_conventions_are_all_answered() {
        let retro = RetroLog {
            broke: vec!["assumed the API was idempotent".to_string()],
            avoid: vec!["skipping the reproduce step".to_string()],
            conventions: vec!["always add --dry-run to destructive commands".to_string()],
            body: String::new(),
        };
        let entry = format_journal_entry(&JournalEntryParams {
            run_id: "2026-07-27-add-greet",
            run_title: "add greet",
            run_profile: "feature",
            retro: &retro,
            branch: Some("milestone-3-ecosystem"),
            when: Some(WHEN),
            tz_offset_secs: Some(0),
        });
        let expected = [
            "## 2026-07-27 14:05 · gate · milestone-3-ecosystem",
            "- **Did:** Completed gate run \"add greet\" (2026-07-27-add-greet, profile feature)",
            "- **Learned:** assumed the API was idempotent",
            "- **Decided:** always add --dry-run to destructive commands",
            "- **Avoid:** skipping the reproduce step",
            "- **Source:** .gate/runs/2026-07-27-add-greet",
            "",
        ]
        .join("\n");
        assert_eq!(entry, expected);
    }

    #[test]
    fn omits_empty_slots_and_the_branch_segment_when_unknown() {
        let retro = RetroLog {
            broke: vec![],
            avoid: vec!["forgetting to check the token expiry edge case".to_string()],
            conventions: vec![],
            body: String::new(),
        };
        let entry = format_journal_entry(&JournalEntryParams {
            run_id: "2026-07-27-fix-login",
            run_title: "fix login",
            run_profile: "bugfix",
            retro: &retro,
            branch: None,
            when: Some(WHEN),
            tz_offset_secs: Some(0),
        });
        let expected = [
            "## 2026-07-27 14:05 · gate",
            "- **Did:** Completed gate run \"fix login\" (2026-07-27-fix-login, profile bugfix)",
            "- **Avoid:** forgetting to check the token expiry edge case",
            "- **Source:** .gate/runs/2026-07-27-fix-login",
            "",
        ]
        .join("\n");
        assert_eq!(entry, expected);
    }

    #[test]
    fn joins_multiple_entries_in_a_slot_with_a_middle_dot() {
        let retro = RetroLog {
            broke: vec![
                "one thing broke".to_string(),
                "another thing broke".to_string(),
            ],
            avoid: vec![],
            conventions: vec![],
            body: String::new(),
        };
        let entry = format_journal_entry(&JournalEntryParams {
            run_id: "r1",
            run_title: "t",
            run_profile: "feature",
            retro: &retro,
            branch: Some("main"),
            when: Some(WHEN),
            tz_offset_secs: Some(0),
        });
        assert!(entry.contains("- **Learned:** one thing broke \u{b7} another thing broke"));
        assert!(!entry.contains("; "));
    }

    #[test]
    fn journal_file_path_pins_the_month_to_utc_even_near_a_month_boundary() {
        // 2026-07-31 23:30 UTC and 2026-08-01 00:30 UTC - the UTC month must
        // win regardless of what local wall-clock time a given TZ would show.
        assert_eq!(
            journal_file_path(1785540600), // Date.UTC(2026, 6, 31, 23, 30)
            ".agnosgram/journal/2026-07.md"
        );
        assert_eq!(
            journal_file_path(1785544200), // Date.UTC(2026, 7, 1, 0, 30)
            ".agnosgram/journal/2026-08.md"
        );
    }

    #[test]
    fn writes_the_fallback_journal_creating_the_months_file_and_appending_the_entry() {
        let root = tmp_repo("fallback-create");
        let retro = RetroLog {
            broke: vec!["x".to_string()],
            avoid: vec![],
            conventions: vec![],
            body: String::new(),
        };
        let entry = format_journal_entry(&JournalEntryParams {
            run_id: "r1",
            run_title: "demo",
            run_profile: "feature",
            retro: &retro,
            branch: Some("main"),
            when: Some(WHEN),
            tz_offset_secs: Some(0),
        });

        let result = write_journal_entry_with(NO_SUCH_BIN, &root, &entry, Some(WHEN)).unwrap();
        assert_eq!(result.method, JournalWriteMethod::Fallback);
        assert_eq!(result.journal_file, ".agnosgram/journal/2026-07.md");

        let abs = root.join(&result.journal_file);
        assert!(abs.exists());
        let content = stdfs::read_to_string(&abs).unwrap();
        assert!(content.contains("## 2026-07-27 14:05 · gate · main"));
        assert!(content.contains("- **Source:** .gate/runs/r1"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn appends_a_second_entry_without_clobbering_the_first() {
        let root = tmp_repo("fallback-append");
        let retro_a = RetroLog {
            broke: vec!["a".to_string()],
            avoid: vec![],
            conventions: vec![],
            body: String::new(),
        };
        let retro_b = RetroLog {
            broke: vec!["b".to_string()],
            avoid: vec![],
            conventions: vec![],
            body: String::new(),
        };
        let first = format_journal_entry(&JournalEntryParams {
            run_id: "r1",
            run_title: "first",
            run_profile: "feature",
            retro: &retro_a,
            branch: Some("main"),
            when: Some(WHEN),
            tz_offset_secs: None,
        });
        let second = format_journal_entry(&JournalEntryParams {
            run_id: "r2",
            run_title: "second",
            run_profile: "feature",
            retro: &retro_b,
            branch: Some("main"),
            when: Some(WHEN),
            tz_offset_secs: None,
        });

        let r1 = write_journal_entry_with(NO_SUCH_BIN, &root, &first, Some(WHEN)).unwrap();
        let r2 = write_journal_entry_with(NO_SUCH_BIN, &root, &second, Some(WHEN)).unwrap();
        assert_eq!(r1.journal_file, r2.journal_file);

        let content = stdfs::read_to_string(root.join(&r1.journal_file)).unwrap();
        assert!(content.contains(".gate/runs/r1"));
        assert!(content.contains(".gate/runs/r2"));
        stdfs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn spawns_the_agnosgram_cli_with_the_entry_on_stdin_and_reports_method_agnosgram_cli() {
        let root = tmp_repo("cli-stub");
        let record_dir = crate::core::testutil::unique_temp_dir("agnosgram-record");
        let record_path = record_dir.join("received.md");
        let stub = record_dir.join("agnosgram");
        stdfs::write(
            &stub,
            format!(
                "#!/bin/sh\nif [ \"$1\" != \"log\" ] || [ \"$2\" != \"--stdin\" ] || [ \"$3\" != \"--agent\" ] || [ \"$4\" != \"gate\" ]; then\n  echo \"unexpected args: $@\" >&2\n  exit 1\nfi\ncat > \"{}\"\nexit 0\n",
                record_path.display()
            ),
        )
        .unwrap();
        let mut perms = stdfs::metadata(&stub).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        stdfs::set_permissions(&stub, perms).unwrap();

        let retro = RetroLog {
            broke: vec!["x".to_string()],
            avoid: vec![],
            conventions: vec![],
            body: String::new(),
        };
        let entry = format_journal_entry(&JournalEntryParams {
            run_id: "r1",
            run_title: "demo",
            run_profile: "feature",
            retro: &retro,
            branch: Some("main"),
            when: Some(WHEN),
            tz_offset_secs: None,
        });

        let result =
            write_journal_entry_with(stub.to_str().unwrap(), &root, &entry, Some(WHEN)).unwrap();
        assert_eq!(result.method, JournalWriteMethod::AgnosgramCli);
        assert_eq!(result.journal_file, ".agnosgram/journal/2026-07.md");
        assert_eq!(stdfs::read_to_string(&record_path).unwrap(), entry);

        stdfs::remove_dir_all(&root).unwrap();
        stdfs::remove_dir_all(&record_dir).unwrap();
    }

    #[test]
    fn journal_contains_run_id_reads_the_journal_file() {
        let root = tmp_repo("contains-run-id");
        let rel = ".agnosgram/journal/2026-01.md";
        stdfs::create_dir_all(root.join(".agnosgram/journal")).unwrap();
        stdfs::write(
            root.join(rel),
            "# Journal\n\n## entry\n- **Source:** .gate/runs/r1\n",
        )
        .unwrap();
        assert!(journal_contains_run_id(&root, rel, "r1"));
        assert!(!journal_contains_run_id(&root, rel, "r2"));
        assert!(!journal_contains_run_id(&root, "nonexistent.md", "r1"));
        stdfs::remove_dir_all(&root).unwrap();
    }
}
