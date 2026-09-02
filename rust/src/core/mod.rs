//! One module per `src/core/*.ts` file, same names (camelCase TS files
//! become snake_case Rust modules, e.g. `stateMachine.ts` ->
//! `state_machine.rs`, matching agnosgram's port convention). Wave 1 fully
//! ports the pure-data modules plus the three new hand-rolled modules
//! (`json`, `yaml`, `sha256`); the process/state modules (`exec`, `git`,
//! `run`, `current`, `trust`, `gitignore_state`, `playbooks`,
//! `embedded_playbooks`) are wave-2 skeletons.

pub mod config;
pub mod current;
pub mod embedded_playbooks;
pub mod exec;
pub mod fsx;
pub mod git;
pub mod gitignore_state;
pub mod glob;
pub mod identity;
pub mod json;
pub mod markers;
pub mod paths;
pub mod playbook_manifest;
pub mod playbooks;
pub mod run;
pub mod sha256;
pub mod state_machine;
pub mod targets;
pub mod trust;
pub mod version;
pub mod yaml;
