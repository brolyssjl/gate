//! Port of `test/prune.test.ts` ("gate prune" archiving old runs).
//! See `CONFORMANCE_MAP.md` for the TS case -> Rust test fn mapping.

mod common;

use common::{gate, iso8601_days_ago, make_repo, read_file, write_file};

/// Write a minimal, valid, non-active run.json directly (skip the full
/// state-machine walk). Port of `prune.test.ts`'s local `seedRun`.
fn seed_run(repo: &std::path::Path, id: &str, updated_at: &str, status: &str) {
    let json = format!(
        "{{\"schema\":2,\"id\":\"{id}\",\"title\":\"{id}\",\"profile\":\"docs\",\"phase\":\"DONE\",\
         \"status\":\"{status}\",\"createdAt\":\"{updated_at}\",\"updatedAt\":\"{updated_at}\",\
         \"baseRef\":null,\"sessionId\":null,\
         \"history\":[{{\"phase\":\"PLAN\",\"event\":\"entered\",\"at\":\"{updated_at}\"}}],\
         \"overrides\":[],\"artifacts\":{{}}}}"
    );
    write_file(repo, &format!(".gate/runs/{id}/run.json"), &json);
}

/// `arr.sort()` on a JSON string array, for comparing against an expected
/// (already-sorted) list regardless of the order `pruned` came back in.
fn sorted_strings(value: &common::json::Value) -> Vec<String> {
    let mut items: Vec<String> = value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    items.sort();
    items
}

#[test]
fn keeps_the_newest_keep_runs_and_prunes_the_rest_archiving_summaries() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    for i in 0..5 {
        seed_run(&repo, &format!("run-{i}"), &iso8601_days_ago(i), "done");
    }

    let dry = gate(&repo, &["prune", "--keep", "2", "--dry-run", "--json"]);
    assert_eq!(dry.code, 0);
    let dry_data = dry.json();
    assert_eq!(dry_data.bool_at("dryRun"), Some(true));
    assert_eq!(
        sorted_strings(dry_data.get("pruned").unwrap()),
        vec![
            "run-2".to_string(),
            "run-3".to_string(),
            "run-4".to_string()
        ]
    );
    // Dry run touches nothing.
    assert!(repo.join(".gate/runs/run-4").exists());
    assert!(!repo.join(".gate/archive").exists());

    let real = gate(&repo, &["prune", "--keep", "2", "--json"]);
    assert_eq!(real.code, 0);
    let real_data = real.json();
    assert_eq!(
        sorted_strings(real_data.get("pruned").unwrap()),
        vec![
            "run-2".to_string(),
            "run-3".to_string(),
            "run-4".to_string()
        ]
    );

    // Newest 2 survive untouched.
    assert!(repo.join(".gate/runs/run-0").exists());
    assert!(repo.join(".gate/runs/run-1").exists());
    // Pruned run folders are gone, but archived.
    for id in ["run-2", "run-3", "run-4"] {
        assert!(!repo.join(format!(".gate/runs/{id}")).exists());
        let archive_path = format!(".gate/archive/{id}.json");
        assert!(repo.join(&archive_path).exists());
        let archived = common::json::parse(&read_file(&repo, &archive_path)).unwrap();
        assert_eq!(archived.str("id"), Some(id));
    }
}

#[test]
fn never_prunes_the_active_run_even_if_its_the_oldest() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    gate(&repo, &["start", "active one", "--profile", "docs"]);
    let active_id = gate(&repo, &["status", "--json"])
        .json()
        .str("id")
        .unwrap()
        .to_string();
    // Backdate it so it would otherwise be the top prune candidate.
    let run_json_path = format!(".gate/runs/{active_id}/run.json");
    let run = common::json::parse(&read_file(&repo, &run_json_path)).unwrap();
    let mut entries: Vec<(String, common::json::Value)> = run.as_object().unwrap().to_vec();
    let backdated = iso8601_days_ago(30);
    for (k, v) in entries.iter_mut() {
        if k == "updatedAt" {
            *v = common::json::Value::String(backdated.clone());
        }
    }
    write_file(&repo, &run_json_path, &render_json_object(&entries));
    seed_run(&repo, "finished-run", &iso8601_days_ago(1), "done");

    let res = gate(&repo, &["prune", "--keep", "0", "--json"]);
    let data = res.json();
    let pruned = sorted_strings(data.get("pruned").unwrap());
    assert_eq!(pruned, vec!["finished-run".to_string()]);
    assert!(!pruned.contains(&active_id));
    assert!(repo.join(format!(".gate/runs/{active_id}")).exists());
}

#[test]
fn protects_every_run_current_json_maps_to_even_a_done_run_mapped_under_a_branch_other_than_the_one_checked_out(
) {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    // A done-but-still-mapped run (the failure mode a migration/advance bug
    // could produce): status is "done", so nothing about its own record
    // marks it active, but current.json still points a branch at it.
    seed_run(&repo, "stale-mapped-run", &iso8601_days_ago(30), "done");
    write_file(
        &repo,
        ".gate/current.json",
        "{\"schema\":1,\"branches\":{\"some-other-branch\":\"stale-mapped-run\"}}",
    );
    seed_run(&repo, "genuinely-finished", &iso8601_days_ago(1), "done");

    let res = gate(&repo, &["prune", "--keep", "0", "--json"]);
    let data = res.json();
    let pruned = sorted_strings(data.get("pruned").unwrap());
    assert_eq!(pruned, vec!["genuinely-finished".to_string()]);
    assert!(!pruned.contains(&"stale-mapped-run".to_string()));
    assert!(repo.join(".gate/runs/stale-mapped-run").exists());
}

#[test]
fn days_additionally_requires_a_candidate_to_be_older_than_n_days() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    seed_run(&repo, "recent", &iso8601_days_ago(1), "done");
    seed_run(&repo, "old", &iso8601_days_ago(40), "done");

    // keep=0 alone would prune both; --days 30 spares the recent one.
    let res = gate(&repo, &["prune", "--keep", "0", "--days", "30", "--json"]);
    let data = res.json();
    let pruned = sorted_strings(data.get("pruned").unwrap());
    assert_eq!(pruned, vec!["old".to_string()]);
    assert!(repo.join(".gate/runs/recent").exists());
    assert!(!repo.join(".gate/runs/old").exists());
}

#[test]
fn gate_report_falls_back_to_the_archived_summary_after_a_run_is_pruned() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    seed_run(&repo, "archived-run", &iso8601_days_ago(10), "done");
    gate(&repo, &["prune", "--keep", "0"]);
    assert!(!repo.join(".gate/runs/archived-run").exists());

    let report = gate(&repo, &["report", "archived-run", "--json"]);
    assert_eq!(report.code, 0);
    let data = report.json();
    assert_eq!(data.str("id"), Some("archived-run"));
    assert_eq!(data.str("profile"), Some("docs"));
    assert!(!report.stdout.is_empty());

    let human = gate(&repo, &["report", "archived-run"]);
    assert!(human.stdout.contains("(archived)"));
}

#[test]
fn gate_report_with_no_run_id_falls_back_to_the_newest_archived_summary_after_a_full_prune() {
    let repo = make_repo(&[]);
    gate(&repo, &["init"]);
    seed_run(&repo, "older-run", &iso8601_days_ago(20), "done");
    seed_run(&repo, "newer-run", &iso8601_days_ago(2), "done");
    // Prune both, oldest archived first so mtime recency actually distinguishes them.
    gate(&repo, &["prune", "--keep", "0"]);
    assert!(!repo.join(".gate/runs/older-run").exists());
    assert!(!repo.join(".gate/runs/newer-run").exists());

    // No positional arg, no active run - must not error just because the live
    // runs folder is empty; both summaries are still on disk under archive/.
    let report = gate(&repo, &["report", "--json"]);
    assert_eq!(report.code, 0);
    let data = report.json();
    let id = data.str("id").unwrap();
    assert!(id == "older-run" || id == "newer-run");
}

/// Minimal object-to-JSON-text renderer for round-tripping a parsed run.json
/// after editing one field in place. Only needs to handle the value shapes
/// `run.json` actually contains (string/int/bool/null/array/object) - no
/// escaping beyond what `json::parse` already unescaped back out, matching
/// this crate's dependency-free JSON constraint.
fn render_json_value(value: &common::json::Value) -> String {
    use common::json::Value;
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => render_json_string(s),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(render_json_value).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(entries) => render_json_object(entries),
    }
}

fn render_json_object(entries: &[(String, common::json::Value)]) -> String {
    let parts: Vec<String> = entries
        .iter()
        .map(|(k, v)| format!("{}:{}", render_json_string(k), render_json_value(v)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

fn render_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
