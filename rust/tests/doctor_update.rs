//! Black-box coverage for `gate doctor`/`gate update` (the self-healing
//! install work) and the SDD-composed adapter body/phase hints they share
//! with `gate init`/`gate adapt`/`gate playbook`/`gate next`. New commands -
//! no TS conformance-suite counterpart to map back to (see
//! `CONFORMANCE_MAP.md`, and `main.rs`'s note on `streak` for the same
//! precedent).

mod common;

use common::{gate, make_repo, write_file};

#[test]
fn doctor_reports_actionable_findings_on_a_freshly_inited_repo_and_exits_1() {
    let repo = make_repo(&[]);
    assert_eq!(gate(&repo, &["init", "--no-adapt"]).code, 0);

    let human = gate(&repo, &["doctor"]);
    assert_eq!(human.code, 1);
    assert!(human.stdout.contains("actionable"));

    let json = gate(&repo, &["doctor", "--json"]);
    assert_eq!(json.code, 1);
    let parsed = json.json();
    assert!(parsed.get("actionable").unwrap().as_i64().unwrap() > 0);
    let adapters = parsed.get("adapters").unwrap().as_array().unwrap();
    assert!(adapters
        .iter()
        .any(|a| a.str("adapter") == Some("claude") && a.str("status") == Some("missing")));
    // `gate init` already materializes every bundled playbook copy - the
    // remaining actionable item on a freshly inited repo is trust: those
    // copies ride in the trust hash (`core::config`), so an init that never
    // ran `gate trust` is still untrusted even with an empty commands block.
    let trust = parsed.get("trust").unwrap();
    assert_eq!(trust.bool_at("trusted"), Some(false));
}

#[test]
fn update_heals_a_freshly_inited_repo_and_a_second_doctor_run_is_clean() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);

    let update = gate(&repo, &["update"]);
    assert_eq!(update.code, 0);
    assert!(update.stdout.contains("created")); // claude/agents adapters, missing after --no-adapt

    assert!(gate(&repo, &["trust"]).code == 0);

    let second_update = gate(&repo, &["update"]);
    assert_eq!(second_update.code, 0);
    assert!(second_update.stdout.contains("nothing to do"));

    let doctor = gate(&repo, &["doctor"]);
    assert_eq!(doctor.code, 0);
    assert!(doctor.stdout.contains("nothing actionable"));
}

#[test]
fn update_never_replaces_a_user_edited_playbook_without_force() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["update"]);
    write_file(&repo, ".gate/playbooks/plan.md", "# my own PLAN playbook\n");

    let update = gate(&repo, &["update"]);
    assert_eq!(update.code, 0);
    assert!(update.stdout.contains("user-edited"));
    assert_eq!(
        common::read_file(&repo, ".gate/playbooks/plan.md"),
        "# my own PLAN playbook\n"
    );

    let doctor = gate(&repo, &["doctor", "--json"]).json();
    let playbooks = doctor.get("playbooks").unwrap().as_array().unwrap();
    let plan = playbooks
        .iter()
        .find(|p| p.str("file") == Some("plan.md"))
        .unwrap();
    assert_eq!(plan.str("status"), Some("user-edited"));
    assert_eq!(plan.str("severity"), Some("info"));

    let forced = gate(&repo, &["update", "--force-playbooks"]);
    assert_eq!(forced.code, 0);
    assert!(forced.stdout.contains("force-replaced"));
    assert_ne!(
        common::read_file(&repo, ".gate/playbooks/plan.md"),
        "# my own PLAN playbook\n"
    );
}

#[test]
fn update_adapt_flag_installs_an_additional_adapter_target() {
    let repo = make_repo(&[]);
    gate(&repo, &["init", "--no-adapt"]);
    let result = gate(&repo, &["update", "--adapt", "cursor"]);
    assert_eq!(result.code, 0);
    assert!(std::path::Path::new(&repo)
        .join(".cursor/rules/gate.mdc")
        .exists());
}

#[test]
fn init_installs_claude_and_agents_by_default_and_no_adapt_skips_them() {
    let repo = make_repo(&[]);
    assert_eq!(gate(&repo, &["init"]).code, 0);
    assert!(std::path::Path::new(&repo).join("CLAUDE.md").exists());
    assert!(std::path::Path::new(&repo).join("AGENTS.md").exists());

    let repo2 = make_repo(&[]);
    assert_eq!(gate(&repo2, &["init", "--no-adapt"]).code, 0);
    assert!(!std::path::Path::new(&repo2).join("CLAUDE.md").exists());
    assert!(!std::path::Path::new(&repo2).join("AGENTS.md").exists());
}

#[test]
fn sdd_present_composes_the_pointer_body_and_doctor_reports_it() {
    let repo = make_repo(&[]);
    write_file(&repo, "openspec/README.md", "seed\n");
    assert_eq!(gate(&repo, &["init"]).code, 0);

    let claude = common::read_file(&repo, "CLAUDE.md");
    assert!(claude.contains("composed with openspec"));
    assert!(claude.contains("fulfilled by openspec's `propose` step"));

    let doctor = gate(&repo, &["doctor", "--json"]).json();
    let sdd = doctor.get("sdd").unwrap();
    assert_eq!(sdd.str("framework"), Some("openspec"));
    assert_eq!(sdd.bool_at("composed"), Some(true));
}

#[test]
fn gate_playbook_at_implement_carries_an_sdd_composition_hint() {
    let repo = make_repo(&[]);
    write_file(&repo, "openspec/README.md", "seed\n");
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["trust"]);
    gate(&repo, &["start", "add feature"]);

    let plan = "---\ngoal: g\nspec: openspec/README.md\nfiles:\n  - a.txt\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n";
    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    write_file(&repo, &format!(".gate/runs/{run_id}/plan.md"), plan);
    gate(&repo, &["approve"]);
    gate(&repo, &["next"]);

    let playbook = gate(&repo, &["playbook"]);
    assert!(playbook.stdout.contains("fulfilled by its `apply` step"));
}

#[test]
fn done_banner_names_the_sdd_closing_step() {
    let repo = make_repo(&[]);
    write_file(&repo, "openspec/README.md", "seed\n");
    gate(&repo, &["init", "--no-adapt"]);
    gate(&repo, &["trust"]);
    gate(&repo, &["start", "docs sweep", "--profile", "docs"]);

    let run_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    write_file(
        &repo,
        &format!(".gate/runs/{run_id}/plan.md"),
        "---\ngoal: g\nspec: openspec/README.md\nfiles:\n  - a.txt\n  - openspec/**\ncriteria:\n  - id: c1\n    text: t\n    verify: manual\n---\n",
    );
    gate(&repo, &["approve"]);
    assert_eq!(gate(&repo, &["next"]).code, 0); // -> IMPLEMENT

    write_file(&repo, "a.txt", "content\n");
    let done = gate(&repo, &["next"]); // IMPLEMENT -> DONE (docs profile)
    assert_eq!(done.code, 0);
    assert!(done.stdout.contains("reached DONE"));
    assert!(done.stdout.contains("openspec archive <change>"));
}
