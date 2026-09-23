//! install.sh's sourcing guard must run `main` when the script is executed
//! directly or piped into bash (`curl ... | bash`, where BASH_SOURCE is
//! unset), and must not run it when the script is sourced for tests.
//! Regression test for the 2026-09-23 hotfix: under `set -u` the unset
//! `BASH_SOURCE[0]` aborted the piped install before `main` ran.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn install_sh() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../install.sh");
    std::fs::read_to_string(path).expect("read install.sh")
}

/// The script with its real `main` replaced by a stub, so the guard can be
/// exercised without touching the network or the user's PATH.
fn script_with_stub_main() -> String {
    let src = install_sh();
    let start = src.find("\nmain() {").expect("main() in install.sh");
    let end = src[start..].find("\n}\n").expect("end of main()") + start + 3;
    format!(
        "{}\nmain() {{ echo \"STUB MAIN RAN\"; }}\n{}",
        &src[..start],
        &src[end..]
    )
}

fn run_bash(args: &[&str], stdin: Option<&str>) -> String {
    let mut child = Command::new("bash")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bash");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    let out = child.wait_with_output().unwrap();
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn piped_into_bash_runs_main() {
    let out = run_bash(&[], Some(&script_with_stub_main()));
    assert!(
        out.contains("STUB MAIN RAN"),
        "piped run did not reach main:\n{out}"
    );
    assert!(
        !out.contains("unbound variable"),
        "guard tripped set -u:\n{out}"
    );
}

#[test]
fn executed_directly_runs_main() {
    let dir = std::env::temp_dir().join(format!(
        "gate-install-guard-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("install.sh");
    std::fs::write(&script, script_with_stub_main()).unwrap();
    let out = run_bash(&[script.to_str().unwrap()], None);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.contains("STUB MAIN RAN"),
        "direct run did not reach main:\n{out}"
    );
}

#[test]
fn sourcing_does_not_run_main() {
    let dir = std::env::temp_dir().join(format!(
        "gate-install-guard-src-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("install.sh");
    std::fs::write(&script, script_with_stub_main()).unwrap();
    let cmd = format!(
        "set -euo pipefail; source '{}'; echo SOURCED_OK",
        script.display()
    );
    let out = run_bash(&["-c", &cmd], None);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.contains("SOURCED_OK"), "sourcing failed:\n{out}");
    assert!(!out.contains("STUB MAIN RAN"), "sourcing ran main:\n{out}");
}
