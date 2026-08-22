//! Skeleton for `src/commands/shared.ts` - in TS a barrel re-export of
//! `cli/output.js` and `cli/context.js` (`emit`, `resolveFormat`,
//! `UsageError`, `GateError`, `requireRoot`, `requireActiveRun`,
//! `ActiveContext`). Rust command modules import directly from
//! `crate::cli::output`/`crate::cli::context` instead of through a barrel
//! file, so this module intentionally has no re-exports - it exists only
//! to keep the 1:1 file mapping with `src/commands/*.ts`.
