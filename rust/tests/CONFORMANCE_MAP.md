# Conformance map: TS -> Rust

Maps every case in the retired TS conformance suite (`test/*.test.ts`, 68
cases across 6 files) to its Rust integration test in `rust/tests/`. All 68
were ported 1:1 - none dropped. Baseline: the original TS suite was run once
against the Rust release binary before any deletion (`GATE_BIN=.../gate
npx vitest run test/cli.e2e.test.ts test/concurrency.test.ts
test/humanReview.test.ts test/prune.test.ts test/guard.test.ts
test/amend.test.ts`) - 68/68 passed, confirming the port's baseline.

This map is deliberately scoped to those ported cases. Suites added after
the TS retirement have no TS counterpart and are intentionally absent -
their file-level doc comments say so in place: `streak.rs` and
`doctor_update.rs` (post-port features), `identity.rs` (issues #29/#33),
`untrusted.rs` (issue #39, untrusted agent-facing inputs), the 2026-09-22
security-audit remediation suites `trust_local_store.rs` (finding 1),
`stored_state_validation.rs` (findings 2 and 4) and `write_containment.rs`
(findings 3 and 10), `install_guard.rs` (the post-release install.sh
sourcing-guard hotfix), and `test_hygiene.rs` (finding 11, a source lint
over the crate's own unit tests).

## test/amend.test.ts -> rust/tests/amend.rs

| TS case | Rust test fn |
|---|---|
| post-PLAN scope widening fails closed until amended (previously accepted with no re-approval) | `post_plan_scope_widening_fails_closed_until_amended` |
| gate amend refuses without a prior approval, and refuses when nothing has drifted | `amend_refuses_without_prior_approval_and_refuses_when_nothing_drifted` |
| gate approve --amend refuses without gate amend having recorded intent first | `approve_amend_refuses_without_gate_amend_having_recorded_intent_first` |
| amend + approve --amend goes green: shows the diff, re-approves, and the drift check clears | `amend_then_approve_amend_goes_green` |
| F2 regression: a doctored approved-plan snapshot is never trusted for the diff | `f2_regression_doctored_approved_plan_snapshot_never_trusted` |
| editing the plan again after `gate amend` invalidates that amendment - approve --amend refuses | `editing_plan_again_after_amend_invalidates_that_amendment` |

## test/cli.e2e.test.ts -> rust/tests/cli_e2e.rs

| TS case | Rust test fn |
|---|---|
| walks a run from PLAN to DONE, refusing every hollow gate | `walks_a_run_from_plan_to_done_refusing_every_hollow_gate` |
| emits a stable --json schema for check | `emits_a_stable_json_schema_for_check` |
| records a human-authorized skip and advances | `records_a_human_authorized_skip_and_advances` |
| registers an artifact with gate log | `registers_an_artifact_with_gate_log` |
| selects the phase set from the --profile flag | `selects_the_phase_set_from_the_profile_flag` |
| rejects an unknown profile | `rejects_an_unknown_profile` |
| targets: --target override wins, and playbook overlays surface for affected targets | `targets_target_override_wins_and_playbook_overlays_surface_for_affected_targets` |
| gate playbook \<phase\> --run honors the named run's targets, not the current branch's own run | `playbook_run_honors_the_named_runs_targets_not_the_current_branchs_own_run` |
| rejects `gate start --target` with an unknown target name | `rejects_gate_start_target_with_an_unknown_target_name` |
| target playbook overlays surface inline at `gate start`, not just the standalone `gate playbook` | `target_playbook_overlays_surface_inline_at_gate_start_not_just_the_standalone_gate_playbook` |
| target playbook overlays surface inline at `gate next`'s phase-entry print | `target_playbook_overlays_surface_inline_at_gate_nexts_phase_entry_print` |
| target playbook overlays surface inline in `gate review`'s emitted rubric | `target_playbook_overlays_surface_inline_in_gate_reviews_emitted_rubric` |
| refuses to reach DONE when a review fix breaks the code (staleness guard) | `refuses_to_reach_done_when_a_review_fix_breaks_the_code_staleness_guard` |
| gate retro syncs the journal (fallback path, hermetic via GATE_AGNOSGRAM_BIN) and is idempotent | `gate_retro_syncs_the_journal_fallback_path_hermetic_and_is_idempotent` |
| gate adapt writes every adapter by default and is idempotent across the CLI | `gate_adapt_writes_every_adapter_by_default_and_is_idempotent_across_the_cli` |
| reports per-run durations, gate failures, and findings | `reports_per_run_durations_gate_failures_and_findings` |
| F1 regression: widening .gitignore to hide an undeclared file cannot pass the scope check | `f1_regression_widening_gitignore_to_hide_an_undeclared_file_cannot_pass_the_scope_check` |
| init/start/status expose stable top-level keys | `json_schema_init_start_status_expose_stable_top_level_keys` |
| check/report/playbook expose stable top-level keys | `json_schema_check_report_playbook_expose_stable_top_level_keys` |

## test/concurrency.test.ts -> rust/tests/concurrency.rs

| TS case | Rust test fn |
|---|---|
| keys runs by branch: independent runs on separate branches, status shows the current one plus others in flight | `keys_runs_by_branch_independent_runs_on_separate_branches_status_shows_the_current_one_plus_others_in_flight` |
| gate start resumes a branch's existing active run instead of erroring | `gate_start_resumes_a_branchs_existing_active_run_instead_of_erroring` |
| gate start refuses to resume on an explicit --profile conflict instead of silently discarding it | `gate_start_refuses_to_resume_on_an_explicit_profile_conflict_instead_of_silently_discarding_it` |
| gate start refuses to resume on an explicit --target conflict instead of silently discarding it | `gate_start_refuses_to_resume_on_an_explicit_target_conflict_instead_of_silently_discarding_it` |
| gate start warns instead of silently discarding a title mismatch when resuming (title alone never blocks) | `gate_start_warns_instead_of_silently_discarding_a_title_mismatch_when_resuming_title_alone_never_blocks` |
| gate start refuses to resume when the plan changed since approval | `gate_start_refuses_to_resume_when_the_plan_changed_since_approval` |
| refuses to start on a detached HEAD (no branch to key the run by) | `refuses_to_start_on_a_detached_head_no_branch_to_key_the_run_by` |
| detached HEAD: gate status reports it without throwing; phase commands require --run | `detached_head_gate_status_reports_it_without_throwing_phase_commands_require_run` |
| detached HEAD: gate report with no run id refuses instead of silently describing another branch's run | `detached_head_gate_report_with_no_run_id_refuses_instead_of_silently_describing_another_branchs_run` |
| --run refuses a non-active (done/abandoned) run instead of letting phase gates pass vacuously | `run_refuses_a_non_active_done_abandoned_run_instead_of_letting_phase_gates_pass_vacuously` |
| --run refuses a run whose recorded branch doesn't match the checked-out branch (would otherwise diff against the wrong tree) | `run_refuses_a_run_whose_recorded_branch_doesnt_match_the_checked_out_branch_would_otherwise_diff_against_the_wrong_tree` |
| migrates the legacy single-run .gate/current pointer into per-branch current.json | `migrates_the_legacy_single_run_gate_current_pointer_into_per_branch_current_json` |
| finishing a migrated legacy run backfills its branch and clears its current.json mapping on DONE | `finishing_a_migrated_legacy_run_backfills_its_branch_and_clears_its_current_json_mapping_on_done` |
| resolves the branch name on an unborn branch (git init, zero commits yet) - distinct from detached HEAD | `resolves_the_branch_name_on_an_unborn_branch_git_init_zero_commits_yet_distinct_from_detached_head` |
| works with no git repo at all: a single implicit key, no branch ambiguity | `works_with_no_git_repo_at_all_a_single_implicit_key_no_branch_ambiguity` |
| fails closed on a corrupt current.json instead of silently discarding every branch's mapping | `fails_closed_on_a_corrupt_current_json_instead_of_silently_discarding_every_branchs_mapping` |
| serializes concurrent `gate start` on the same branch: exactly one run wins, current.json never corrupts | `serializes_concurrent_gate_start_on_the_same_branch_exactly_one_run_wins_current_json_never_corrupts` |
| gate status degrades gracefully (and self-heals) on a dangling mapping instead of crashing with 'run not found' | `gate_status_degrades_gracefully_and_self_heals_on_a_dangling_mapping_instead_of_crashing_with_run_not_found` |

## test/humanReview.test.ts -> rust/tests/human_review.rs

| TS case | Rust test fn |
|---|---|
| records a waived blocker with a rationale, satisfying the same REVIEW gate as an agent review | `records_a_waived_blocker_with_a_rationale_satisfying_the_same_review_gate_as_an_agent_review` |
| blocks the REVIEW gate on an open blocker finding, same as an agent-recorded one | `blocks_the_review_gate_on_an_open_blocker_finding_same_as_an_agent_recorded_one` |
| records multiple findings and requires a reviewer name before writing review.md | `records_multiple_findings_and_requires_a_reviewer_name_before_writing_review_md` |
| reuses an existing packet unless --fresh is passed, matching the non-human review command | `reuses_an_existing_packet_unless_fresh_is_passed_matching_the_non_human_review_command` |
| errors clearly instead of hanging when input ends before the review is recorded | `errors_clearly_instead_of_hanging_when_input_ends_before_the_review_is_recorded` |
| defaults to a non-colliding finding id when an earlier one was deleted by hand (only f2 remains) | `defaults_to_a_non_colliding_finding_id_when_an_earlier_one_was_deleted_by_hand_only_f2_remains` |
| re-prompts when the reviewer explicitly types a finding id that's already used | `re_prompts_when_the_reviewer_explicitly_types_a_finding_id_thats_already_used` |

## test/prune.test.ts -> rust/tests/prune.rs

| TS case | Rust test fn |
|---|---|
| keeps the newest --keep runs and prunes the rest, archiving summaries | `keeps_the_newest_keep_runs_and_prunes_the_rest_archiving_summaries` |
| never prunes the active run, even if it's the oldest | `never_prunes_the_active_run_even_if_its_the_oldest` |
| protects every run current.json maps to, even a done run mapped under a branch other than the one checked out | `protects_every_run_current_json_maps_to_even_a_done_run_mapped_under_a_branch_other_than_the_one_checked_out` |
| --days additionally requires a candidate to be older than N days | `days_additionally_requires_a_candidate_to_be_older_than_n_days` |
| gate report falls back to the archived summary after a run is pruned | `gate_report_falls_back_to_the_archived_summary_after_a_run_is_pruned` |
| gate report with no run id falls back to the newest archived summary after a full prune | `gate_report_with_no_run_id_falls_back_to_the_newest_archived_summary_after_a_full_prune` |

## test/guard.test.ts -> rust/tests/guard.rs

| TS case | Rust test fn |
|---|---|
| install writes an executable pre-commit hook carrying the gate-guard marker | `install_writes_an_executable_pre_commit_hook_carrying_the_gate_guard_marker` |
| install is idempotent: a second install reports already-installed instead of re-wrapping | `install_is_idempotent_a_second_install_reports_already_installed_instead_of_re_wrapping` |
| install backs up and chains a pre-existing foreign hook | `install_backs_up_and_chains_a_pre_existing_foreign_hook` |
| uninstall restores a backed-up foreign hook | `uninstall_restores_a_backed_up_foreign_hook` |
| uninstall removes a hook it installed with nothing to restore | `uninstall_removes_a_hook_it_installed_with_nothing_to_restore` |
| uninstall refuses to touch a pre-commit hook gate didn't install | `uninstall_refuses_to_touch_a_pre_commit_hook_gate_didnt_install` |
| gate guard run blocks with no active run on the branch | `guard_run_blocks_with_no_active_run_on_the_branch` |
| gate guard run informs rather than misleadingly blocks on an unparseable/missing plan.md (e.g. after `gate skip PLAN`) | `guard_run_informs_rather_than_misleadingly_blocks_on_an_unparseable_missing_plan_md` |
| gate guard run blocks while still in PLAN | `guard_run_blocks_while_still_in_plan` |
| gate guard run blocks staged files outside the declared plan scope, passes for in-scope files | `guard_run_blocks_staged_files_outside_the_declared_plan_scope_passes_for_in_scope_files` |
| end-to-end: an installed hook blocks `git commit` for out-of-scope staged files, and --no-verify / GATE_GUARD=0 bypass it | `end_to_end_installed_hook_blocks_git_commit_for_out_of_scope_staged_files_and_no_verify_gate_guard_0_bypass_it` |
| GATE_GUARD=0 disables only the gate check - a chained pre-existing hook still runs | `gate_guard_0_disables_only_the_gate_check_a_chained_pre_existing_hook_still_runs` |
