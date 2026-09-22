//! Shared test harness for the conformance suite (port of `test/helpers.ts`
//! and `test/globalSetup.ts`). Std-only, matching the crate's
//! zero-dependency constraint - no `assert_cmd`, no dev-dependencies.
//!
//! `cargo test` builds the `gate` binary itself before running integration
//! tests (the analogue of `globalSetup.ts`'s one-time `npm run build`), so
//! there is no separate build step to replicate here.
//!
//! Each `tests/*.rs` file is compiled as its own crate and only uses the
//! subset of this harness it needs, so `cargo clippy --all-targets` sees
//! "unused" functions per-target that another target does use - allow
//! dead_code crate-wide here rather than sprinkling per-item allows.
#![allow(dead_code)]

pub mod json;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Resolve how to invoke the `gate` binary under test: `$GATE_BIN` when set
/// (conformance mode against any binary implementing the CLI contract these
/// black-box tests assert, e.g. a downloaded release artifact) - otherwise
/// the binary `cargo test` just built for this crate. Mirrors
/// `helpers.ts`'s `gateCommand`.
pub fn gate_bin() -> String {
    std::env::var("GATE_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_gate").to_string())
}

pub struct GateOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl GateOutput {
    /// `JSON.parse(stdout)` - panics on invalid JSON, same as the TS
    /// helper's `json()` would throw.
    pub fn json(&self) -> json::Value {
        json::parse(&self.stdout).unwrap_or_else(|e| {
            panic!(
                "invalid JSON on stdout: {e}\nstdout was: {:?}\nstderr was: {:?}",
                self.stdout, self.stderr
            )
        })
    }
}

#[derive(Default)]
pub struct GateOpts<'a> {
    pub env: &'a [(&'a str, &'a str)],
    pub input: Option<&'a str>,
}

/// A `GATE_CONFIG_DIR` shared by every spawned `gate` process in this test
/// binary, created once and reused for the life of the process - never the
/// real `$HOME/.config/gate` (security audit 2026-09-22, finding 1: trust
/// is machine-local now, and this suite must not read or write the
/// machine's real trust store). Trust records are keyed by a hash of the
/// canonicalized project root (`core::trust`), so distinct repos under this
/// one shared directory never collide; a test that needs its own isolated
/// config dir (to assert on the store's exact contents, or to simulate two
/// different machines) passes `GATE_CONFIG_DIR` explicitly via `GateOpts`,
/// which wins over this default.
fn default_gate_config_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = unique_dir("gate-config-home");
        fs::create_dir_all(&dir).unwrap();
        dir
    })
}

/// Spawn the `gate` CLI under test and capture its result. See `gate_bin`
/// re: `$GATE_BIN`. Port of `helpers.ts`'s `gate()`.
pub fn gate(cwd: &Path, args: &[&str]) -> GateOutput {
    gate_opts(cwd, args, GateOpts::default())
}

/// Same as `gate`, with extra env vars and/or piped stdin. Port of
/// `helpers.ts`'s `gate()` called with `{ env, input }`.
pub fn gate_opts(cwd: &Path, args: &[&str], opts: GateOpts) -> GateOutput {
    let mut cmd = Command::new(gate_bin());
    cmd.args(args)
        .current_dir(cwd)
        .env("GATE_CONFIG_DIR", default_gate_config_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in opts.env {
        cmd.env(k, v);
    }
    if opts.input.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let mut child = cmd.spawn().expect("failed to spawn gate binary");
    if let Some(input) = opts.input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .expect("failed to write stdin");
    }
    let output = child
        .wait_with_output()
        .expect("failed to wait on gate binary");
    GateOutput {
        code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Spawn the CLI under test without waiting, for launching several
/// invocations genuinely concurrently (see `concurrency.rs`). Port of
/// `helpers.ts`'s `gateAsync` - Rust has no event loop to await on, so the
/// concurrency itself comes from spawning every child before waiting on any
/// of them (`spawn_all` + `wait_all` below), not from an async fn.
pub fn gate_spawn(cwd: &Path, args: &[&str]) -> Child {
    Command::new(gate_bin())
        .args(args)
        .current_dir(cwd)
        .env("GATE_CONFIG_DIR", default_gate_config_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn gate binary")
}

pub fn wait(child: Child) -> GateOutput {
    let output = child
        .wait_with_output()
        .expect("failed to wait on gate binary");
    GateOutput {
        code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Shell command line that invokes the CLI under test, for embedding in a
/// shim script (e.g. a `gate` on PATH so an installed git hook resolves it
/// in tests). Mirrors `gate_bin`'s `$GATE_BIN` resolution. Port of
/// `helpers.ts`'s `gateShellCommand`.
pub fn gate_shell_command() -> String {
    format!("\"{}\"", gate_bin())
}

static UNIQUE: AtomicU64 = AtomicU64::new(0);

fn unique_dir(prefix: &str) -> PathBuf {
    let n = UNIQUE.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{n}", std::process::id()))
}

/// Create an isolated temp dir outside of any git repo. Port of the
/// `mkdtempSync(join(tmpdir(), "gate-no-git-"))` pattern used directly in
/// `concurrency.test.ts` for the no-git and unborn-branch cases.
pub fn make_temp_dir(prefix: &str) -> PathBuf {
    let dir = unique_dir(prefix);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("failed to run git");
    assert!(status.success(), "git {args:?} failed in {cwd:?}");
}

/// Run git and return trimmed stdout, panicking on failure. Port of the
/// small `git()` helpers each conformance file defines locally for calls
/// like `git(["branch", "--show-current"])`.
pub fn git_out(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to run git");
    assert!(
        output.status.success(),
        "git {args:?} failed in {cwd:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Run git without asserting success, capturing code/stdout/stderr - used by
/// `guard.rs` to assert a commit was blocked/bypassed. Port of
/// `guard.test.ts`'s local non-throwing `git()` helper.
pub fn git_run(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> GateOutput {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("failed to run git");
    GateOutput {
        code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Create an isolated temp git repo with an initial commit. Returns its
/// path. Port of `helpers.ts`'s `makeRepo`.
pub fn make_repo(files: &[(&str, &str)]) -> PathBuf {
    let dir = unique_dir("gate-test");
    fs::create_dir_all(&dir).unwrap();
    run_git(&dir, &["init", "-q"]);
    run_git(&dir, &["config", "user.email", "test@test.co"]);
    run_git(&dir, &["config", "user.name", "test"]);
    run_git(&dir, &["config", "commit.gpgsign", "false"]);
    write_file(&dir, "README.md", "seed\n");
    for (path, content) in files {
        write_file(&dir, path, content);
    }
    run_git(&dir, &["add", "-A"]);
    run_git(&dir, &["commit", "-qm", "init"]);
    dir
}

pub fn write_file(root: &Path, rel: &str, content: &str) {
    let abs = root.join(rel);
    if let Some(parent) = abs.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(abs, content).unwrap();
}

pub fn read_file(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
}

/// `new Date(Date.now() - n * 24*60*60*1000).toISOString()` - std has no
/// calendar formatter, so this hand-rolls days-since-epoch -> civil date
/// (Howard Hinnant's `civil_from_days`) rather than pull in a date crate.
/// Used by `prune.rs`'s `daysAgo` to seed run.json timestamps.
pub fn iso8601_days_ago(n: i64) -> String {
    let now = std::time::SystemTime::now();
    let target = now - std::time::Duration::from_secs((n.max(0) as u64) * 86_400);
    let dur = target.duration_since(std::time::UNIX_EPOCH).unwrap();
    let total_secs = dur.as_secs() as i64;
    let millis = dur.subsec_millis();
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let h = secs_of_day / 3600;
    let mi = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{millis:03}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
