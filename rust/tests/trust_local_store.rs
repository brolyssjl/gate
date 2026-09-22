//! Conformance test for security audit finding 1 (2026-09-22, CRITICAL): a
//! cloned repo shipped its own `.gate/trust.json`, so `is_commands_trusted`
//! read the hash straight out of the repo and returned true on the victim's
//! very first `gate check` - the `commands:` block ran via `sh -c` with no
//! `gate trust` ever invoked on the victim's machine. See
//! `docs/integrity.md#command-trust-tofu` and `core::trust`.
//!
//! Fix: trust is a property of (this machine, this checkout), never of the
//! repo. The trust record moves to a machine-local store under
//! `GATE_CONFIG_DIR` (or `$XDG_CONFIG_HOME/gate`, or `$HOME/.config/gate`),
//! keyed by a hash of the canonicalized project root. A repo-tracked
//! `.gate/trust.json` is inert for the trust decision - at most an
//! advisory record of what the repo's authors approved.

mod common;

use common::{gate_opts, make_repo, make_temp_dir, write_file, GateOpts};

fn env1<'a>(key: &'a str, value: &'a str) -> [(&'a str, &'a str); 1] {
    [(key, value)]
}

/// Reproduces the finding end to end, the same way the audit did: a repo
/// with a `commands:` entry whose only effect is a canary file, plus a
/// committed `.gate/trust.json` whose hash matches that config exactly (an
/// attacker's own `gate trust`, committed alongside `config.yml`). On a
/// fresh machine (an empty `GATE_CONFIG_DIR`, standing in for a victim who
/// has never run `gate trust`), the command must never run and the gate
/// must report untrusted. Then, running `gate trust` for real on that same
/// machine/checkout must pass and must write only under `GATE_CONFIG_DIR`,
/// never into the repo.
#[test]
fn a_cloned_repos_tracked_trust_json_does_not_grant_trust_on_a_fresh_machine() {
    let repo = make_repo(&[]);

    // Attacker's machine: any config dir works, it only needs to produce
    // the commands hash for the config below.
    let attacker_dir = make_temp_dir("gate-config-attacker");
    let attacker_env = env1("GATE_CONFIG_DIR", attacker_dir.to_str().unwrap());

    let init = gate_opts(
        &repo,
        &["init", "--no-adapt"],
        GateOpts {
            env: &attacker_env,
            input: None,
        },
    );
    assert_eq!(init.code, 0, "stderr: {}", init.stderr);

    // A canary written OUTSIDE the repo - proof of code execution, not just
    // a file inside the checkout that scope discipline might catch anyway.
    let outside = make_temp_dir("gate-rce-canary-dir");
    let canary = outside.join("RCE_PROOF.txt");
    let config_path = repo.join(".gate/config.yml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    std::fs::write(
        &config_path,
        config.replacen(
            "commands:",
            &format!("commands:\n  build: \"touch {}\"", canary.display()),
            1,
        ),
    )
    .unwrap();

    // Attacker trusts their own copy and commits the resulting record
    // alongside config.yml - mirrors the audit's exact reproduction.
    let attacker_trust = gate_opts(
        &repo,
        &["trust", "--json"],
        GateOpts {
            env: &attacker_env,
            input: None,
        },
    );
    assert_eq!(attacker_trust.code, 0, "stderr: {}", attacker_trust.stderr);
    let commands_hash = attacker_trust
        .json()
        .str("commandsHash")
        .unwrap()
        .to_string();
    write_file(
        &repo,
        ".gate/trust.json",
        &format!(
            "{{\n  \"commandsHash\": \"{commands_hash}\",\n  \"trustedAt\": \"2026-01-01T00:00:00.000Z\",\n  \"trustedBy\": \"attacker\",\n  \"coverageVersion\": 2\n}}\n"
        ),
    );
    let repo_trust_before = std::fs::read_to_string(repo.join(".gate/trust.json")).unwrap();

    // Victim clones the repo onto a fresh machine: empty GATE_CONFIG_DIR,
    // no local trust record has ever been written here.
    let victim_dir = make_temp_dir("gate-config-victim");
    let victim_env = env1("GATE_CONFIG_DIR", victim_dir.to_str().unwrap());
    assert!(
        !victim_dir.join("trust").exists(),
        "victim's config dir must start with no trust store at all"
    );

    let start = gate_opts(
        &repo,
        &["start", "victim clone"],
        GateOpts {
            env: &victim_env,
            input: None,
        },
    );
    assert_eq!(start.code, 0, "stderr: {}", start.stderr);
    let run_id = gate_opts(
        &repo,
        &["status", "--json"],
        GateOpts {
            env: &victim_env,
            input: None,
        },
    )
    .json()
    .str("id")
    .unwrap()
    .to_string();

    write_file(
        &repo,
        &format!(".gate/runs/{run_id}/plan.md"),
        "---\ngoal: g\nfiles:\n  - a.txt\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan\n",
    );
    let approve = gate_opts(
        &repo,
        &["approve"],
        GateOpts {
            env: &victim_env,
            input: None,
        },
    );
    assert_eq!(approve.code, 0, "stderr: {}", approve.stderr);
    let entered = gate_opts(
        &repo,
        &["next"],
        GateOpts {
            env: &victim_env,
            input: None,
        },
    );
    assert_eq!(entered.code, 0, "stderr: {}", entered.stderr);

    write_file(&repo, "a.txt", "changed\n");

    // The moment of truth: on the victim's very first `gate check`, with
    // the repo's committed (and hash-matching) trust.json in place, the
    // build command must NOT run and the gate must report untrusted.
    let checked = gate_opts(
        &repo,
        &["check"],
        GateOpts {
            env: &victim_env,
            input: None,
        },
    );
    assert_eq!(
        checked.code, 1,
        "IMPLEMENT gate must fail while untrusted on this machine\nstdout: {}",
        checked.stdout
    );
    assert!(checked.stdout.contains("not trusted"), "{}", checked.stdout);
    assert!(checked.stdout.contains("gate trust"), "{}", checked.stdout);
    assert!(
        !canary.exists(),
        "the build command must never have run - the repo's tracked trust.json must not grant trust"
    );

    // The repo's tracked trust.json is untouched by any of the above - it
    // is read by nothing and written by nothing in this flow.
    assert_eq!(
        std::fs::read_to_string(repo.join(".gate/trust.json")).unwrap(),
        repo_trust_before
    );

    // Now the victim does the right thing: `gate trust` on their own
    // machine, for this checkout. It must pass, and it must write only
    // under GATE_CONFIG_DIR - never into the repo.
    let victim_trust = gate_opts(
        &repo,
        &["trust"],
        GateOpts {
            env: &victim_env,
            input: None,
        },
    );
    assert_eq!(victim_trust.code, 0, "stderr: {}", victim_trust.stderr);

    let trust_store = victim_dir.join("trust");
    assert!(
        trust_store.is_dir(),
        "gate trust must create the local trust store under GATE_CONFIG_DIR"
    );
    let records: Vec<_> = std::fs::read_dir(&trust_store)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(
        records.len(),
        1,
        "expected exactly one trust record under the local store, got {records:?}"
    );
    let record_text = std::fs::read_to_string(&records[0]).unwrap();
    assert!(record_text.contains(&commands_hash));
    assert!(
        record_text.contains(repo.to_str().unwrap())
            || record_text.contains(&repo.canonicalize().unwrap().to_string_lossy().into_owned()),
        "trust record should carry the canonical project root for human inspection: {record_text}"
    );

    // The repo's tracked trust.json is still untouched - `gate trust` never
    // writes there any more.
    assert_eq!(
        std::fs::read_to_string(repo.join(".gate/trust.json")).unwrap(),
        repo_trust_before
    );

    // And the legitimate flow now actually works end to end: the build
    // command runs and the canary appears.
    let now_trusted_check = gate_opts(
        &repo,
        &["check"],
        GateOpts {
            env: &victim_env,
            input: None,
        },
    );
    assert_eq!(
        now_trusted_check.code, 0,
        "stdout: {}",
        now_trusted_check.stdout
    );
    assert!(
        canary.exists(),
        "build command should now run and create the canary"
    );
}
