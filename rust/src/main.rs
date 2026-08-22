//! Port of `src/cli.ts`: argv dispatch, help/version, exit codes. Mirrors
//! the TS entry point exactly - see `docs/rust-port.md`'s exactness
//! contract.
//!
//! Wave 1 stubs every command's body (`commands::*::run`) with
//! `Err(UserError::new("not yet ported"))`; later waves replace bodies in
//! place without touching this file's dispatch table or signatures.
//!
//! Wave 1 scaffolds every `core`/`gates`/`commands`/`artifacts`/
//! `integrations`/`serialize` module ahead of the commands that will call
//! into them (see `docs/rust-port.md`'s wave plan): most of their exports
//! are unused until a later wave wires them up. Allow dead code for that
//! transitional state rather than defeat clippy's `-D warnings` gate with
//! artificial uses - each module still carries its own `#[cfg(test)]`
//! coverage, exercised by `cargo test`.
#![allow(dead_code)]

mod adapters;
mod artifacts;
mod cli;
mod commands;
mod core;
mod gates;
mod integrations;
mod serialize;

use cli::args::parse_args;
use cli::output::UserError;
use core::version::read_version;

/// Byte-for-byte copy of `src/cli.ts`'s `HELP` template literal, with
/// `${ADAPTER_KEYS.join(", ")}` already resolved to its frozen value
/// (`claude, claude-skill, cursor, cline, windsurf, agents` - see
/// `src/adapters/index.ts` / `adapters::adapter_keys()`). Captured directly
/// from `node dist/cli.js --help`'s real output rather than transcribed by
/// hand, to eliminate escaping-transcription risk (backticks/quotes inside
/// the TS template literal).
const HELP: &str = r#"gate - an agent-agnostic quality harness (umpire, not a driver).

Usage: gate <command> [options]

Commands:
  adapt [adapter...]      Write/refresh agent config pointer blocks (default:
                          all of claude, claude-skill, cursor, cline, windsurf, agents)
  init [--refresh]        Scaffold .gate/, infer commands, detect integrations
  trust [--check] [--by]  Approve the config commands block (TOFU); required
                          before gates will execute build/test/lint
  start "<title>"         Create a run, enter PLAN, print the plan playbook.
                          One active run per branch: a branch with a run
                          already in flight is resumed, not restarted
    [--profile <p>]       Phases to run: feature|bugfix|refactor|docs (default feature)
    [--target <a,b>]      Override target resolution (comma-separated names);
                          wins over file-based resolution for this run
  approve [--by] [--reason] [--run <id>]  Record PLAN sign-off, bound to the plan's content
    [--amend]              Re-approve a plan that drifted after approval,
                          recording a new hash for the delta `gate amend`
                          showed (requires `gate amend` to have run first)
  amend [--by] [--run <id>]  Show the diff between plan.md and the approved
                          snapshot and record intent to re-approve it; does
                          not itself re-approve - run `gate approve --amend`
                          after
  status                  Active run for the current branch, plus any other
                          branches with a run in flight
  check [--run <id>]      Run the current gate; exit code = verdict
  next [--run <id>]       Check + advance on pass; on fail, print what's missing
  review [--fresh] [--by] [--run <id>]  Emit a self-contained review packet
                          (REVIEW phase); --fresh regenerates it from the
                          current code
    [--human]              Walk the rubric via terminal prompts instead of
                          handing the packet to another session (solo-dev
                          review mode; readline only, no new dependencies)
  retro [--run <id>]      Sync retro.md into the Agnosgram journal (RETRO
                          phase); no-op without a .agnosgram/ store
  report [<run-id>]       Per-run summary: durations, gate failures, findings
                          (falls back to an archived summary after prune)
  prune [--keep n] [--days n] [--dry-run]  Archive non-active runs past the
                          retention window to .gate/archive/, then remove them
  skip <phase> --reason [--by] [--run <id>]  Human-authorized skip of the
                          current phase (recorded with who and why; shown by
                          `gate report`)
  log <file> [--run <id>] Register an artifact against the current phase
  playbook [phase] [--run <id>]  Print the active playbook for a phase
  guard install|uninstall  Manage an opt-in .git/hooks/pre-commit guard;
                          backs up and chains any existing hook. Cheap
                          deterministic checks only (active run, staged files
                          in plan scope, not still in PLAN) - never installed
                          by `init`, never load-bearing. Bypass per commit
                          with --no-verify, or always with GATE_GUARD=0

Global options:
  --json                  Machine-readable JSON output
  --format json|toon      Choose the serializer (toon: uniform arrays only)
  --run <id>              Act on a specific run instead of resolving the
                          current branch's active run (required on a detached
                          HEAD, where there's no branch to resolve from)
  -h, --help              Show this help
  -v, --version           Show version

The agent loop is two commands: `gate playbook` (what do I do?) → `gate next`
(am I done?). All state lives on disk under .gate/."#;

type CommandFn = fn(Vec<String>) -> Result<(), UserError>;

fn dispatch(command: &str) -> Option<CommandFn> {
    Some(match command {
        "init" => commands::init::run,
        "adapt" => commands::adapt::run,
        "trust" => commands::trust::run,
        "start" => commands::start::run,
        "approve" => commands::approve::run,
        "amend" => commands::amend::run,
        "status" => commands::status::run,
        "check" => commands::check::run,
        "next" => commands::next::run,
        "review" => commands::review::run,
        "retro" => commands::retro::run,
        "report" => commands::report::run,
        "prune" => commands::prune::run,
        "skip" => commands::skip::run,
        "log" => commands::log::run,
        "playbook" => commands::playbook::run,
        "guard" => commands::guard::run,
        _ => return None,
    })
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv);

    if args.flags.is_true("help")
        || args.flags.is_true("h")
        || args.command.as_deref() == Some("help")
    {
        println!("{HELP}");
        return;
    }
    if args.flags.is_true("version")
        || args.flags.is_true("v")
        || args.command.as_deref() == Some("version")
    {
        println!("{}", read_version());
        return;
    }
    let Some(command) = &args.command else {
        println!("{HELP}");
        return;
    };

    let Some(handler) = dispatch(command) else {
        eprint!("gate: unknown command \"{command}\"\n\n{HELP}\n");
        std::process::exit(2);
    };

    // Every command's own positionals/flags parsing happens inside `run`;
    // wave 1 stubs ignore argv entirely. Later waves pass the full
    // remaining argv (everything after the command token) exactly as
    // TS's `args` (the whole `ParsedArgs`) is threaded to each `cmd*`
    // handler today.
    let rest: Vec<String> = argv.into_iter().skip(1).collect();
    if let Err(err) = handler(rest) {
        report_error(&err);
    }
}

/// Port of `cli.ts`'s `reportError`: `UsageError` -> exit 2, `GateError` ->
/// its own `exitCode` (default 1), any other error -> exit 1. All three
/// print identically (`gate: <message>`) - only the exit code differs.
fn report_error(err: &UserError) {
    eprintln!("gate: {}", err.message());
    std::process::exit(err.exit_code());
}
