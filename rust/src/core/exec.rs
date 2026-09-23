//! Port of `src/core/exec.ts` (`runCommand`, `ExecResult`): run a configured
//! command string through the shell, from `cwd`. Gate executes commands
//! from config.yml - the same trust class as npm scripts (proposal §9) -
//! and only after `gate trust` has pinned them. `env` entries are layered
//! over the process env so gates can hand runners well-known paths
//! (`GATE_TEST_REPORT`).
//!
//! TS always runs through a shell (`spawnSync(command, { shell: true, ... })`);
//! there is no no-shell path in `core/exec.ts` itself (that's `core/git.ts`,
//! which spawns `git` directly with an argv array; see `core/git.rs`).
//! Mirrored here with `sh -c <command>` on Unix (Node's `shell: true` uses
//! `/bin/sh -c` on POSIX platforms) - Gate ships darwin/linux binaries only
//! (see `docs/rust-port.md`), so no `cmd.exe` branch is needed.

use std::collections::HashMap;
use std::process::Command;

/// Result of running a command: exit code plus captured stdout/stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run `command` through `sh -c`, from `cwd`. `env` entries are layered over
/// the inherited process environment (added/overridden, never a full
/// replacement) - matching TS's `{ ...process.env, ...env }`.
///
/// Exit-code mapping mirrors TS's `res.status ?? (res.error ? 127 : 1)`:
/// the shell's own exit code when it ran, `127` when the command could not
/// be spawned at all (`res.error` set - e.g. `sh` itself missing), `1` for
/// any other case with no status (namely: killed by a signal, `res.status
/// === null` with no `error` - Node's own fallback for "something odd
/// happened but we don't have a code").
pub fn run_command(command: &str, cwd: &str, env: Option<&HashMap<String, String>>) -> ExecResult {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command).current_dir(cwd);
    if let Some(extra) = env {
        for (k, v) in extra {
            cmd.env(k, v);
        }
    }
    match cmd.output() {
        Ok(output) => ExecResult {
            code: output.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(_) => ExecResult {
            code: 127,
            stdout: String::new(),
            stderr: String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_a_zero_exit_code_on_success() {
        let dir = crate::core::testutil::unique_temp_dir("exec-rs-ok");
        let res = run_command("echo hello", dir.to_str().unwrap(), None);
        assert_eq!(res.code, 0);
        assert_eq!(res.stdout, "hello\n");
        assert_eq!(res.stderr, "");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn captures_stderr_and_a_nonzero_exit_code_on_failure() {
        // An explicit absolute cwd, not "." - a relative cwd made this test
        // sensitive to unrelated tests' tempdir churn under parallel runs.
        let dir = crate::core::testutil::unique_temp_dir("exec-rs-fail");
        let res = run_command("echo oops 1>&2; exit 3", dir.to_str().unwrap(), None);
        assert_eq!(res.code, 3);
        assert_eq!(res.stderr, "oops\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn runs_from_the_given_cwd() {
        let dir = crate::core::testutil::unique_temp_dir("exec-rs-cwd");
        let res = run_command("pwd", dir.to_str().unwrap(), None);
        // Compare canonicalized paths - macOS temp dirs are often a symlink
        // (/tmp -> /private/tmp), and `pwd` reports the resolved path.
        let expected = std::fs::canonicalize(&dir).unwrap();
        let actual = std::fs::canonicalize(res.stdout.trim()).unwrap();
        assert_eq!(actual, expected);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn layers_extra_env_vars_over_the_inherited_environment() {
        let mut env = HashMap::new();
        env.insert(
            "GATE_TEST_REPORT".to_string(),
            "/tmp/report.json".to_string(),
        );
        let dir = crate::core::testutil::unique_temp_dir("exec-rs-env");
        let res = run_command("echo $GATE_TEST_REPORT", dir.to_str().unwrap(), Some(&env));
        assert_eq!(res.stdout, "/tmp/report.json\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
