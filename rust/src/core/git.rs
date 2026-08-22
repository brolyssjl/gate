//! Skeleton for `src/core/git.ts` (`changedFiles`, `isGateBookkeeping`,
//! `treeFingerprint`, branch/diff helpers). Ported in wave 2 (see
//! `docs/rust-port.md`) via `std::process::Command` spawning `git`.
//! `core::targets::resolve_display_targets` depends on `changed_files` and
//! `is_gate_bookkeeping` and is deferred until this module lands - see the
//! TODO at the bottom of `core/targets.rs`.
