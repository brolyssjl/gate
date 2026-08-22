//! One module per `src/commands/*.ts` file, same names. Wave 1 stubs every
//! dispatched command's body with `Err(UserError::new("not yet ported"))`;
//! later waves replace bodies in place without touching this file or
//! `main.rs`'s dispatch signatures (`fn run(argv: Vec<String>) -> Result<(),
//! UserError>`). The non-dispatch helper modules (`advance`, `gate_run`,
//! `infer_stack`, `shared`) are plain skeletons, ported alongside whichever
//! command(s) import them.

pub mod adapt;
pub mod advance;
pub mod amend;
pub mod approve;
pub mod check;
pub mod gate_run;
pub mod guard;
pub mod infer_stack;
pub mod init;
pub mod log;
pub mod next;
pub mod playbook;
pub mod prune;
pub mod report;
pub mod retro;
pub mod review;
pub mod shared;
pub mod skip;
pub mod start;
pub mod status;
pub mod trust;
