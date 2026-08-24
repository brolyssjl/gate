//! Port of `test/cli.e2e.test.ts` ("gate CLI end-to-end" + "gate --json
//! schema"). See `CONFORMANCE_MAP.md` for the TS case -> Rust test fn
//! mapping.

mod common;

use common::{gate, gate_opts, git_out, make_repo, write_file, GateOpts};

const PLAN: &str = "---\ngoal: Add greet\napproved: true\nfiles:\n  - greet.js\n  - test.js\ncriteria:\n  - id: c1\n    text: \"greets by name\"\n    verify: \"test: greets by name\"\n---\n# Plan\n";

fn today_prefix() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    // Reuse the same civil-date math as `common::iso8601_days_ago` (0 days ago).
    let full = common::iso8601_days_ago(0);
    let _ = dur;
    full[..10].to_string()
}

#[test]
fn walks_a_run_from_plan_to_done_refusing_every_hollow_gate() {
    let repo = make_repo(&[(
        "package.json",
        r#"{"name":"fx","scripts":{"test":"node test.js"}}"#,
    )]);

    assert_eq!(gate(&repo, &["init"]).code, 0);
    assert_eq!(gate(&repo, &["trust"]).code, 0); // approve the inferred commands
    assert_eq!(gate(&repo, &["start", "add greet"]).code, 0);

    // PLAN gate fails on the empty scaffold.
    assert_eq!(gate(&repo, &["check"]).code, 1);

    // Write a real plan; it still fails until a separate `gate approve`.
    // Run id is date-based; discover it via status --json instead of hardcoding.
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    write_file(&repo, &format!(".gate/runs/{run_id}/plan.md"), PLAN);
    assert_eq!(gate(&repo, &["check"]).code, 1); // schema ok now, but not approved
    assert_eq!(gate(&repo, &["approve"]).code, 0);
    assert_eq!(gate(&repo, &["next"]).code, 0);
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().str("phase"),
        Some("IMPLEMENT")
    );

    // IMPLEMENT: empty diff fails; in-scope code passes.
    assert_eq!(gate(&repo, &["check"]).code, 1);
    write_file(
        &repo,
        "greet.js",
        "module.exports.greet = (n) => 'Hello, ' + n;\n",
    );
    assert_eq!(gate(&repo, &["next"]).code, 0);

    // TEST: red suite fails.
    write_file(
        &repo,
        "test.js",
        "const {greet}=require('./greet');const ok=greet('Sam')==='Bye';console.log(JSON.stringify({tests:[{name:'greets by name',status:ok?'passed':'failed'}]}));process.exit(ok?0:1)",
    );
    assert_eq!(gate(&repo, &["check"]).code, 1);

    // Green suite -> TEST passes and the run enters REVIEW (feature profile).
    write_file(
        &repo,
        "test.js",
        "const {greet}=require('./greet');const ok=greet('Sam')==='Hello, Sam';console.log(JSON.stringify({tests:[{name:'greets by name',status:ok?'passed':'failed'}]}));process.exit(ok?0:1)",
    );
    assert_eq!(gate(&repo, &["next"]).code, 0);
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().str("phase"),
        Some("REVIEW")
    );

    // REVIEW: blocks until a fresh packet is emitted, and the untouched
    // scaffold must NOT pass - a reviewer has to sign review.md.
    assert_eq!(gate(&repo, &["check"]).code, 1);
    assert_eq!(gate(&repo, &["review", "--fresh"]).code, 0);
    assert_eq!(gate(&repo, &["next"]).code, 1);
    let review_run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    write_file(
        &repo,
        &format!(".gate/runs/{review_run_id}/review.md"),
        "---\nreviewer: fresh-eyes\nfindings: []\n---\n# Review\n",
    );
    assert_eq!(gate(&repo, &["next"]).code, 0);
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().str("phase"),
        Some("RETRO")
    );

    // RETRO: the empty scaffold has no substance and must not pass; a filled-in
    // retro (no .agnosgram/ store in this fixture repo, so no journal sync is
    // required) reaches DONE.
    assert_eq!(gate(&repo, &["check"]).code, 1);
    write_file(
        &repo,
        &format!(".gate/runs/{review_run_id}/retro.md"),
        "---\nbroke: []\navoid:\n  - \"Do not skip the reproduce step\"\nconventions: []\n---\n# Retro\n",
    );
    assert_eq!(gate(&repo, &["next"]).code, 0);

    let done = gate(&repo, &["status", "--json"]).json();
    assert_eq!(done.bool_at("active"), Some(false));
}

#[test]
fn emits_a_stable_json_schema_for_check() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(&repo, &["start", "schema snap"]);
    let res = gate(&repo, &["check", "--json"]);
    let parsed = res.json();
    assert_eq!(parsed.sorted_keys(), vec!["checks", "ok", "phase"]);
    assert_eq!(parsed.str("phase"), Some("PLAN"));
    assert_eq!(parsed.bool_at("ok"), Some(false));
    assert!(parsed.get("checks").unwrap().as_array().is_some());
    let check0 = &parsed.get("checks").unwrap().as_array().unwrap()[0];
    assert_eq!(check0.sorted_keys(), vec!["detail", "name", "ok"]);
}

#[test]
fn records_a_human_authorized_skip_and_advances() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(&repo, &["start", "skip demo"]);

    assert_eq!(gate(&repo, &["skip", "PLAN"]).code, 2); // --reason required
    assert_eq!(
        gate(&repo, &["skip", "PLAN", "--reason", "trivial doc tweak"]).code,
        0
    );

    let status = gate(&repo, &["status", "--json"]).json();
    assert_eq!(status.str("phase"), Some("IMPLEMENT"));
    let run_id = "skip demo"
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let run_id = collapse_dashes(&run_id);
    let run_json_text = common::read_file(
        &repo,
        &format!(".gate/runs/{}-{run_id}/run.json", today_prefix()),
    );
    let run = common::json::parse(&run_json_text).unwrap();
    let overrides = run.get("overrides").unwrap().as_array().unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].str("phase"), Some("PLAN"));
    assert_eq!(overrides[0].str("reason"), Some("trivial doc tweak"));
}

/// `"skip demo".replace(/[^a-z0-9]+/gi, "-").toLowerCase()` collapses runs of
/// non-alphanumerics to a single dash; the char-by-char map above produces
/// one dash per non-alphanumeric char, so collapse consecutive dashes here.
fn collapse_dashes(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in s.chars() {
        if c == '-' {
            if !last_dash {
                out.push(c);
            }
            last_dash = true;
        } else {
            out.push(c);
            last_dash = false;
        }
    }
    out
}

#[test]
fn registers_an_artifact_with_gate_log() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(&repo, &["start", "log demo"]);
    write_file(&repo, "notes.txt", "hello\n");
    assert_eq!(gate(&repo, &["log", "notes.txt"]).code, 0);
    let status = gate(&repo, &["status", "--json"]).json();
    let artifacts = status.get("artifacts").unwrap().as_array().unwrap();
    assert!(artifacts.iter().any(|a| a.as_str() == Some("notes.txt")));
}

#[test]
fn selects_the_phase_set_from_the_profile_flag() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);

    // bugfix routes through DEBUG (and skips IMPLEMENT); it scaffolds debug-log.md.
    assert_eq!(
        gate(&repo, &["start", "fix login", "--profile", "bugfix"]).code,
        0
    );
    let status = gate(&repo, &["status", "--json"]).json();
    assert_eq!(status.str("profile"), Some("bugfix"));
    let phases: Vec<&str> = status
        .get("phases")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        phases,
        vec!["PLAN", "DEBUG", "TEST", "REVIEW", "RETRO", "DONE"]
    );
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    assert!(repo
        .join(format!(".gate/runs/{run_id}/debug-log.md"))
        .exists());
}

#[test]
fn rejects_an_unknown_profile() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    assert_eq!(gate(&repo, &["start", "x", "--profile", "bogus"]).code, 2);
}

#[test]
fn targets_target_override_wins_and_playbook_overlays_surface_for_affected_targets() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    write_file(
        &repo,
        ".gate/playbooks/test.web.md",
        "# Web overlay\n\nRun the visual regression suite.\n",
    );
    write_file(
        &repo,
        ".gate/config.yml",
        "commands: {}\nthresholds: {}\ntargets:\n  web:\n    match: [\"apps/web/**\"]\n    playbooks: { test: \".gate/playbooks/test.web.md\" }\n  api:\n    match: [\"apps/api/**\"]\nphases: {}\nintegrations: { agnosgram: off, sdd: off }\n",
    );
    // The target playbook overlay must be trusted before it takes effect.
    assert_eq!(gate(&repo, &["trust"]).code, 0);

    assert_eq!(
        gate(&repo, &["start", "target override demo", "--target", "web"]).code,
        0
    );
    let playbook = gate(&repo, &["playbook", "TEST", "--json"]).json();
    let text = playbook.str("playbook").unwrap();
    assert!(text.contains("## Target overlay: web"));
    assert!(text.contains("Run the visual regression suite."));
}

#[test]
fn playbook_run_honors_the_named_runs_targets_not_the_current_branchs_own_run() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    write_file(
        &repo,
        ".gate/playbooks/test.web.md",
        "# Web overlay\n\nWEB_OVERLAY_MARKER\n",
    );
    write_file(
        &repo,
        ".gate/playbooks/test.api.md",
        "# Api overlay\n\nAPI_OVERLAY_MARKER\n",
    );
    write_file(
        &repo,
        ".gate/config.yml",
        "commands: {}\nthresholds: {}\ntargets:\n  web:\n    match: [\"apps/web/**\"]\n    playbooks: { test: \".gate/playbooks/test.web.md\" }\n  api:\n    match: [\"apps/api/**\"]\n    playbooks: { test: \".gate/playbooks/test.api.md\" }\nphases: {}\nintegrations: { agnosgram: off, sdd: off }\n",
    );
    // Both target playbook overlays must be trusted before they take effect.
    assert_eq!(gate(&repo, &["trust"]).code, 0);
    let main_branch = git_out(&repo, &["branch", "--show-current"]);

    assert_eq!(
        gate(&repo, &["start", "api run", "--target", "api"]).code,
        0
    );
    let api_run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();

    git_out(&repo, &["checkout", "-b", "web-branch"]);
    assert_eq!(
        gate(&repo, &["start", "web run", "--target", "web"]).code,
        0
    );
    let web_run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();

    git_out(&repo, &["checkout", &main_branch]);
    // Current branch (main) is the api run - the default (no --run) path.
    let default_playbook = gate(&repo, &["playbook", "TEST", "--json"]).json();
    let default_text = default_playbook.str("playbook").unwrap();
    assert!(default_text.contains("API_OVERLAY_MARKER"));
    assert!(!default_text.contains("WEB_OVERLAY_MARKER"));

    // --run must override that default, even though it names a run filed
    // under a completely different branch.
    let via_run = gate(&repo, &["playbook", "TEST", "--run", &web_run_id, "--json"]).json();
    let via_run_text = via_run.str("playbook").unwrap();
    assert!(via_run_text.contains("WEB_OVERLAY_MARKER"));
    assert!(!via_run_text.contains("API_OVERLAY_MARKER"));

    assert_ne!(api_run_id, web_run_id);
}

#[test]
fn rejects_gate_start_target_with_an_unknown_target_name() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    write_file(
        &repo,
        ".gate/config.yml",
        "commands: {}\nthresholds: {}\ntargets:\n  api:\n    match: [\"apps/api/**\"]\nphases: {}\nintegrations: { agnosgram: off, sdd: off }\n",
    );
    let res = gate(
        &repo,
        &["start", "bad target demo", "--target", "does-not-exist"],
    );
    assert_eq!(res.code, 2); // UsageError
    assert!(!repo.join(".gate/current").exists()); // no run created
}

#[test]
fn target_playbook_overlays_surface_inline_at_gate_start_not_just_the_standalone_gate_playbook() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    write_file(
        &repo,
        ".gate/playbooks/plan.api.md",
        "# PLAN overlay\n\nCite the api spec.\n",
    );
    write_file(
        &repo,
        ".gate/config.yml",
        "commands: {}\nthresholds: {}\ntargets:\n  api:\n    match: [\"apps/api/**\"]\n    playbooks: { plan: .gate/playbooks/plan.api.md }\nphases: {}\nintegrations: { agnosgram: off, sdd: off }\n",
    );
    // The target playbook overlay must be trusted before it takes effect.
    assert_eq!(gate(&repo, &["trust"]).code, 0);

    let started = gate(&repo, &["start", "api overlay demo", "--target", "api"]);
    assert_eq!(started.code, 0);
    assert!(started.stdout.contains("## Target overlay: api"));
    assert!(started.stdout.contains("Cite the api spec."));
}

#[test]
fn target_playbook_overlays_surface_inline_at_gate_nexts_phase_entry_print() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    write_file(
        &repo,
        ".gate/playbooks/test.api.md",
        "# TEST overlay\n\nRun the api contract suite.\n",
    );
    write_file(
        &repo,
        ".gate/config.yml",
        "commands: {}\nthresholds: {}\ntargets:\n  api:\n    match: [\"apps/api/**\"]\n    playbooks: { test: .gate/playbooks/test.api.md }\nphases: {}\nintegrations: { agnosgram: off, sdd: off }\n",
    );
    // The target playbook overlay must be trusted before it takes effect.
    assert_eq!(gate(&repo, &["trust"]).code, 0);
    gate(&repo, &["start", "api overlay demo", "--target", "api"]);
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    write_file(
        &repo,
        &format!(".gate/runs/{run_id}/plan.md"),
        "---\ngoal: g\nfiles:\n  - apps/api/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n# Plan\n",
    );
    gate(&repo, &["approve", "--by", "human"]);
    assert_eq!(gate(&repo, &["next"]).code, 0); // PLAN -> IMPLEMENT
    write_file(&repo, "apps/api/x.py", "changed\n");
    let entered_test = gate(&repo, &["next"]); // IMPLEMENT -> TEST
    assert_eq!(entered_test.code, 0);
    assert!(entered_test.stdout.contains("## Target overlay: api"));
    assert!(entered_test.stdout.contains("Run the api contract suite."));
}

#[test]
fn target_playbook_overlays_surface_inline_in_gate_reviews_emitted_rubric() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    write_file(
        &repo,
        ".gate/playbooks/review.api.md",
        "# REVIEW overlay\n\nCheck the api error envelope.\n",
    );
    write_file(
        &repo,
        ".gate/config.yml",
        "commands: {}\nthresholds: {}\ntargets:\n  api:\n    match: [\"apps/api/**\"]\n    playbooks: { review: .gate/playbooks/review.api.md }\nphases: {}\nintegrations: { agnosgram: off, sdd: off }\n",
    );
    // The target playbook overlay must be trusted before it takes effect.
    assert_eq!(gate(&repo, &["trust"]).code, 0);
    gate(&repo, &["start", "api overlay demo", "--target", "api"]);
    assert_eq!(gate(&repo, &["skip", "PLAN", "--reason", "test"]).code, 0);
    assert_eq!(
        gate(&repo, &["skip", "IMPLEMENT", "--reason", "test"]).code,
        0
    );
    assert_eq!(gate(&repo, &["skip", "TEST", "--reason", "test"]).code, 0);
    let review = gate(&repo, &["review", "--fresh", "--json"]).json();
    let rubric = review.str("rubric").unwrap();
    assert!(rubric.contains("## Target overlay: api"));
    assert!(rubric.contains("Check the api error envelope."));
}

/// A target playbook overlay declared in config.yml is refused (never
/// appended) until a human runs `gate trust` - the untrusted config change
/// must not silently start steering the agent.
#[test]
fn untrusted_target_playbook_overlay_is_refused_until_gate_trust_runs() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    write_file(
        &repo,
        ".gate/playbooks/plan.api.md",
        "# PLAN overlay\n\nAPI_OVERLAY_MARKER\n",
    );
    write_file(
        &repo,
        ".gate/config.yml",
        "commands: {}\nthresholds: {}\ntargets:\n  api:\n    match: [\"apps/api/**\"]\n    playbooks: { plan: .gate/playbooks/plan.api.md }\nphases: {}\nintegrations: { agnosgram: off, sdd: off }\n",
    );
    // No `gate trust` yet: the overlay must be refused, not appended, in the
    // playbook `gate start` prints inline.
    let started = gate(
        &repo,
        &["start", "untrusted overlay demo", "--target", "api"],
    );
    assert_eq!(started.code, 0);
    assert!(!started.stdout.contains("## Target overlay: api"));
    assert!(!started.stdout.contains("API_OVERLAY_MARKER"));
    assert!(started.stdout.contains("UNTRUSTED PLAYBOOK IGNORED"));
    assert!(started.stdout.contains("gate trust"));
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();

    // Trusting it makes the overlay take effect for the same run.
    assert_eq!(gate(&repo, &["trust"]).code, 0);
    let trusted = gate(&repo, &["playbook", "PLAN", "--run", &run_id, "--json"]).json();
    let trusted_text = trusted.str("playbook").unwrap();
    assert!(trusted_text.contains("## Target overlay: api"));
    assert!(trusted_text.contains("API_OVERLAY_MARKER"));
}

/// A `.gate/playbooks/<phase>.md` override is refused (falls back to the
/// bundled/embedded default) until a human runs `gate trust`, and editing
/// an already-trusted override again without re-trusting refuses it again.
#[test]
fn untrusted_dot_gate_playbooks_override_is_refused_until_gate_trust_runs() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    write_file(&repo, ".gate/playbooks/plan.md", "# CUSTOM_PLAN_MARKER\n");

    let refused = gate(&repo, &["playbook", "PLAN", "--json"]).json();
    let refused_text = refused.str("playbook").unwrap();
    assert!(!refused_text.contains("CUSTOM_PLAN_MARKER"));
    assert!(refused_text.contains("UNTRUSTED PLAYBOOK IGNORED"));
    assert!(refused_text.contains(".gate/playbooks/plan.md"));
    assert!(refused_text.contains("gate trust"));

    assert_eq!(gate(&repo, &["trust"]).code, 0);
    let trusted = gate(&repo, &["playbook", "PLAN", "--json"]).json();
    assert_eq!(trusted.str("playbook").unwrap(), "# CUSTOM_PLAN_MARKER\n");

    // Editing it again without re-trusting is refused again.
    write_file(&repo, ".gate/playbooks/plan.md", "# TAMPERED_MARKER\n");
    let tampered = gate(&repo, &["playbook", "PLAN", "--json"]).json();
    let tampered_text = tampered.str("playbook").unwrap();
    assert!(!tampered_text.contains("TAMPERED_MARKER"));
    assert!(tampered_text.contains("UNTRUSTED PLAYBOOK IGNORED"));
}

#[test]
fn refuses_to_reach_done_when_a_review_fix_breaks_the_code_staleness_guard() {
    let repo = make_repo(&[
        ("package.json", r#"{"name":"fx","scripts":{"test":"node test.js"}}"#),
        ("greet.js", "module.exports = (n) => 'Hello, ' + n\n"),
        (
            "test.js",
            "const ok = require('./greet')('Sam') === 'Hello, Sam';\nconsole.log(JSON.stringify({tests:[{name:'greets by name',status:ok?'passed':'failed'}]}));\nprocess.exit(ok ? 0 : 1)\n",
        ),
    ]);
    gate(&repo, &["init"]);
    gate(&repo, &["trust"]);
    gate(&repo, &["start", "demo"]);
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    write_file(
        &repo,
        &format!(".gate/runs/{run_id}/plan.md"),
        "---\ngoal: demo\nfiles:\n  - \"greet.js\"\n  - \"test.js\"\ncriteria:\n  - id: c1\n    text: greets\n    verify: \"test: greets by name\"\n---\n# Plan\n",
    );
    gate(&repo, &["approve", "--by", "human"]);
    assert_eq!(gate(&repo, &["next"]).code, 0); // PLAN -> IMPLEMENT
    write_file(
        &repo,
        "greet.js",
        "module.exports = (n) => 'Hello, ' + n // touched\n",
    );
    assert_eq!(gate(&repo, &["next"]).code, 0); // IMPLEMENT -> TEST
    assert_eq!(gate(&repo, &["next"]).code, 0); // TEST -> REVIEW
    let review = gate(&repo, &["review", "--fresh", "--json"]);
    assert_eq!(
        review.json().sorted_keys(),
        vec![
            "diff",
            "findingsFile",
            "packet",
            "phase",
            "plan",
            "regenerated",
            "requestedBy",
            "rubric",
            "treeHash"
        ]
    );

    // Reviewer files a blocker; the implementer "fixes" it by breaking the
    // suite and flips the finding to resolved.
    write_file(
        &repo,
        &format!(".gate/runs/{run_id}/review.md"),
        "---\nreviewer: fresh-eyes\nfindings:\n  - id: f1\n    severity: blocker\n    status: resolved\n    note: crash on null\n---\n",
    );
    write_file(&repo, "greet.js", "throw new Error('boom')\n");

    // Stale packet: the reviewer never saw the "fix".
    assert_eq!(gate(&repo, &["next"]).code, 1);
    // Without --fresh an existing packet is not re-baselined.
    gate(&repo, &["review"]);
    assert_eq!(gate(&repo, &["next"]).code, 1);
    // Even with a fresh packet, re-verification catches the red suite - the
    // 3rd consecutive REVIEW failure, tripping the default failure-streak
    // cap.
    gate(&repo, &["review", "--fresh"]);
    assert_eq!(gate(&repo, &["next"]).code, 1);

    // The cap is now open: even a would-be-passing `next` refuses to
    // evaluate at all until a human explicitly resets the streak.
    let blocked = gate(&repo, &["next"]);
    assert_eq!(blocked.code, 3);
    assert!(blocked.stderr.contains("3 consecutive failures"));
    assert_eq!(
        gate(
            &repo,
            &[
                "streak",
                "reset",
                "--reason",
                "human reviewed, retry warranted"
            ]
        )
        .code,
        0
    );

    // A real fix, re-packeted, goes green and enters RETRO.
    write_file(&repo, "greet.js", "module.exports = (n) => 'Hello, ' + n\n");
    gate(&repo, &["review", "--fresh"]);
    assert_eq!(gate(&repo, &["next"]).code, 0);
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().str("phase"),
        Some("RETRO")
    );
    write_file(
        &repo,
        &format!(".gate/runs/{run_id}/retro.md"),
        "---\nbroke:\n  - \"Fixing a blocker by breaking the build almost shipped\"\navoid: []\nconventions: []\n---\n# Retro\n",
    );
    assert_eq!(gate(&repo, &["next"]).code, 0);
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().bool_at("active"),
        Some(false)
    );
}

#[test]
fn gate_retro_syncs_the_journal_fallback_path_hermetic_and_is_idempotent() {
    let repo = make_repo(&[]);
    write_file(&repo, ".agnosgram/config.yml", "version: 1\n");
    // F3: point at a path that can't resolve to a real binary, so this
    // exercises the ENOENT fallback regardless of what's actually on PATH.
    let env = [(
        "GATE_AGNOSGRAM_BIN",
        "/nonexistent/gate-agnosgram-test-stub",
    )];
    gate(&repo, &["init"]);
    gate(&repo, &["start", "retro sync demo"]);
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    // Walk PLAN -> RETRO the short way: skip every gated phase up to RETRO.
    assert_eq!(gate(&repo, &["skip", "PLAN", "--reason", "test"]).code, 0);
    assert_eq!(
        gate(&repo, &["skip", "IMPLEMENT", "--reason", "test"]).code,
        0
    );
    assert_eq!(gate(&repo, &["skip", "TEST", "--reason", "test"]).code, 0);
    assert_eq!(gate(&repo, &["skip", "REVIEW", "--reason", "test"]).code, 0);
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().str("phase"),
        Some("RETRO")
    );

    // No substance yet: gate retro refuses.
    assert_eq!(
        gate_opts(
            &repo,
            &["retro"],
            GateOpts {
                env: &env,
                input: None
            }
        )
        .code,
        1
    );

    write_file(
        &repo,
        &format!(".gate/runs/{run_id}/retro.md"),
        "---\nbroke: []\navoid:\n  - \"Do not skip reproduce\"\nconventions: []\n---\n# Retro\n",
    );
    let synced = gate_opts(
        &repo,
        &["retro", "--json"],
        GateOpts {
            env: &env,
            input: None,
        },
    );
    assert_eq!(synced.code, 0);
    let synced_data = synced.json();
    assert_eq!(synced_data.bool_at("synced"), Some(true));
    assert_eq!(synced_data.str("method"), Some("fallback"));
    let journal_file = synced_data.str("journalFile").unwrap().to_string();
    let journal = common::read_file(&repo, &journal_file);
    assert!(journal.contains(&run_id));
    assert!(journal.contains("- **Avoid:** Do not skip reproduce"));

    // Idempotent: re-running does not duplicate the entry - byte-identical file.
    let again = gate_opts(
        &repo,
        &["retro", "--json"],
        GateOpts {
            env: &env,
            input: None,
        },
    )
    .json();
    assert_eq!(again.bool_at("alreadySynced"), Some(true));
    let journal_after = common::read_file(&repo, &journal_file);
    assert_eq!(journal_after, journal);

    assert_eq!(gate(&repo, &["next"]).code, 0);
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().bool_at("active"),
        Some(false)
    );
}

#[test]
fn gate_adapt_writes_every_adapter_by_default_and_is_idempotent_across_the_cli() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);

    let first = gate(&repo, &["adapt", "--json"]);
    assert_eq!(first.code, 0);
    let first_data = first.json();
    let adapters = first_data.get("adapters").unwrap().as_array().unwrap();
    assert!(adapters.len() >= 6);
    assert!(adapters.iter().all(|a| a.str("action") == Some("created")));
    for a in adapters {
        assert!(repo.join(a.str("path").unwrap()).exists());
    }

    let second = gate(&repo, &["adapt", "--json"]);
    let second_data = second.json();
    let second_adapters = second_data.get("adapters").unwrap().as_array().unwrap();
    assert!(second_adapters
        .iter()
        .all(|a| a.str("action") == Some("unchanged")));

    assert_eq!(gate(&repo, &["adapt", "cursor", "--json"]).code, 0);
    assert_eq!(gate(&repo, &["adapt", "not-a-real-adapter"]).code, 2);
}

#[test]
fn reports_per_run_durations_gate_failures_and_findings() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(&repo, &["start", "report demo"]);
    // One failed advancement attempt (empty plan) is recorded for the report.
    assert_eq!(gate(&repo, &["next"]).code, 1);

    let report = gate(&repo, &["report", "--json"]).json();
    assert_eq!(report.str("profile"), Some("feature"));
    assert!(report.get("gateFailures").unwrap().as_i64().unwrap() >= 1);
    let phases = report.get("phases").unwrap().as_array().unwrap();
    assert!(phases.iter().any(|p| p.str("phase") == Some("PLAN")));
    assert!(report.get("findings").unwrap().is_null()); // no review.md yet
}

#[test]
fn f1_regression_widening_gitignore_to_hide_an_undeclared_file_cannot_pass_the_scope_check() {
    // Attack this review finding described: since `.gitignore` was
    // unconditionally exempt from "touched", an agent could append an
    // ignore pattern to it, which makes git itself stop reporting whatever
    // it now matches as untracked - hiding an undeclared file from every
    // git-based diff Gate takes, including the scope check - while the
    // .gitignore edit itself sailed through unflagged. The fix: only a
    // .gitignore that's byte-for-byte what `gate init` wrote is exempt: any
    // other edit (this one included) must itself show up as touched and
    // undeclared, failing the gate closed exactly as it does on main.
    let repo = make_repo(&[("package.json", r#"{"name":"fx"}"#)]);
    assert_eq!(gate(&repo, &["init"]).code, 0);
    assert_eq!(gate(&repo, &["trust"]).code, 0);
    let started = gate(&repo, &["start", "attack demo", "--json"]).json();
    let plan_path = started.str("plan").unwrap().to_string();
    write_file(
        &repo,
        &plan_path,
        "---\ngoal: legit change\nfiles:\n  - legit.txt\ncriteria:\n  - id: c1\n    text: it works\n    verify: \"manual\"\n---\n",
    );
    assert_eq!(gate(&repo, &["approve"]).code, 0);
    assert_eq!(gate(&repo, &["next"]).code, 0); // PLAN -> IMPLEMENT

    write_file(&repo, "legit.txt", "hi\n");
    // The attack: widen .gitignore, then drop an undeclared file under the
    // newly-ignored path.
    let gitignore_content = common::read_file(&repo, ".gitignore");
    write_file(
        &repo,
        ".gitignore",
        &format!("{gitignore_content}\npayload/\n"),
    );
    write_file(&repo, "payload/evil.js", "// undeclared\n");

    let check = gate(&repo, &["check", "--json"]);
    assert_eq!(check.code, 1);
    let data = check.json();
    let checks = data.get("checks").unwrap().as_array().unwrap();
    let scope_check = checks
        .iter()
        .find(|c| c.str("name") == Some("implement.scope"))
        .unwrap();
    assert_eq!(scope_check.bool_at("ok"), Some(false));
    assert!(scope_check.str("detail").unwrap().contains(".gitignore"));

    // A genuinely untouched, gate-authored .gitignore is still exempt - the
    // legit change alone (no tampering, payload/ cleaned up) still passes.
    write_file(
        &repo,
        ".gitignore",
        &gitignore_content.replace("\npayload/\n", ""),
    );
    std::fs::remove_dir_all(repo.join("payload")).ok();
    assert_eq!(gate(&repo, &["check", "--json"]).code, 0);
}

#[test]
fn json_schema_init_start_status_expose_stable_top_level_keys() {
    let repo = make_repo(&[(
        "package.json",
        r#"{"name":"fx","scripts":{"test":"node -e 0"}}"#,
    )]);
    gate(&repo, &["init"]);
    gate(&repo, &["trust"]);

    assert_eq!(
        gate(&repo, &["init", "--json"]).json().sorted_keys(),
        vec![
            "detected",
            "gitignoreUpdated",
            "initialized",
            "refreshed",
            "root"
        ]
    );
    assert_eq!(
        gate(&repo, &["start", "schema demo", "--json"])
            .json()
            .sorted_keys(),
        vec!["branch", "hints", "id", "phase", "phases", "plan", "profile"]
    );
    assert_eq!(
        gate(&repo, &["status", "--json"]).json().sorted_keys(),
        vec![
            "active",
            "artifacts",
            "baseRef",
            "branch",
            "id",
            "nextAction",
            "others",
            "phase",
            "phases",
            "profile",
            "sessionId",
            "status",
            "title"
        ]
    );
}

#[test]
fn json_schema_check_report_playbook_expose_stable_top_level_keys() {
    let repo = make_repo(&[(
        "package.json",
        r#"{"name":"fx","scripts":{"test":"node -e 0"}}"#,
    )]);
    gate(&repo, &["init"]);
    gate(&repo, &["trust"]);
    gate(&repo, &["start", "schema demo"]);

    assert_eq!(
        gate(&repo, &["check", "--json"]).json().sorted_keys(),
        vec!["checks", "ok", "phase"]
    );
    assert_eq!(
        gate(&repo, &["report", "--json"]).json().sorted_keys(),
        vec![
            "artifacts",
            "findings",
            "gateFailures",
            "id",
            "overrides",
            "phase",
            "phases",
            "profile",
            "status",
            "title",
            "totalSeconds"
        ]
    );
    assert_eq!(
        gate(&repo, &["playbook", "PLAN", "--json"])
            .json()
            .sorted_keys(),
        vec!["hints", "phase", "playbook"]
    );
}

/// `gate report <id>` must reject a path-traversal / absolute run id
/// before it ever reaches `run_paths`/`archive_path` - `gate report
/// ../../x` must not be able to escape `.gate/runs/`.
#[test]
fn gate_report_rejects_a_path_traversal_run_id() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);

    for bad in ["../../etc/passwd", "..", "a/../../b", "a/b", "/etc/passwd"] {
        let res = gate(&repo, &["report", bad]);
        assert_eq!(res.code, 2, "expected usage error for {bad:?}");
        assert!(res.stderr.contains("invalid run id") || res.stderr.contains("empty"));
    }

    // A normal, gate-generated run id is unaffected.
    gate(
        &repo,
        &["start", "path traversal demo", "--profile", "docs"],
    );
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    let ok = gate(&repo, &["report", &run_id, "--json"]);
    assert_eq!(ok.code, 0);
    assert_eq!(ok.json().str("id"), Some(run_id.as_str()));
}

/// The shared `--run <id>` flag (every phase command, resolved via
/// `cli/context.rs`'s `require_active_run`) must reject the same
/// traversal/absolute shapes, not just `gate report`'s positional.
#[test]
fn gate_run_flag_rejects_a_path_traversal_run_id() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(&repo, &["start", "run flag demo", "--profile", "docs"]);

    for bad in ["../../etc/passwd", "..", "a/b"] {
        let res = gate(&repo, &["check", "--run", bad]);
        assert_eq!(res.code, 2, "expected usage error for {bad:?}");
        assert!(res.stderr.contains("invalid run id"));
    }
}

/// `gate playbook <phase> --run <id>` goes through a second, independent
/// `--run` resolution path (`commands/playbook.rs`'s
/// `execute_explicit_phase`, not `require_active_run`) - must reject the
/// same shapes.
#[test]
fn gate_playbook_run_flag_rejects_a_path_traversal_run_id() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);

    let res = gate(&repo, &["playbook", "PLAN", "--run", "../../etc/passwd"]);
    assert_eq!(res.code, 2);
    assert!(res.stderr.contains("invalid run id"));
}
