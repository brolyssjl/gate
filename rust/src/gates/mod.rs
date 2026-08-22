//! Skeleton for `src/gates/index.ts` (`hasGate`, `runGate`, dispatching to
//! the per-phase gate for `ctx.run.phase`). Ported in wave 3 (see
//! `docs/rust-port.md`), once `core::run::Run`/`core::state_machine::Phase`
//! wiring and every per-phase gate module below are in place.
//!
//! One module per `src/gates/*.ts` file, same names.

pub mod coverage;
pub mod debug;
pub mod implement;
pub mod plan;
pub mod plan_drift;
pub mod retro;
pub mod review;
pub mod test;
pub mod test_report;
pub mod types;
