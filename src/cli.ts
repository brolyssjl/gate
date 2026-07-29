#!/usr/bin/env node
import { parseArgs } from "./cli/args.js";
import { GateError, UsageError } from "./cli/output.js";
import { readVersion } from "./core/version.js";
import { cmdInit } from "./commands/init.js";
import { cmdStart } from "./commands/start.js";
import { cmdStatus } from "./commands/status.js";
import { cmdCheck } from "./commands/check.js";
import { cmdNext } from "./commands/next.js";
import { cmdPlaybook } from "./commands/playbook.js";
import { cmdTrust } from "./commands/trust.js";
import { cmdApprove } from "./commands/approve.js";
import { cmdSkip } from "./commands/skip.js";
import { cmdLog } from "./commands/log.js";
import { cmdReview } from "./commands/review.js";
import { cmdReport } from "./commands/report.js";
import { cmdRetro } from "./commands/retro.js";
import { cmdPrune } from "./commands/prune.js";
import { cmdAdapt } from "./commands/adapt.js";
import { ADAPTER_KEYS } from "./adapters/index.js";

const HELP = `gate - an agent-agnostic quality harness (umpire, not a driver).

Usage: gate <command> [options]

Commands:
  adapt [adapter...]      Write/refresh agent config pointer blocks (default:
                          all of ${ADAPTER_KEYS.join(", ")})
  init [--refresh]        Scaffold .gate/, infer commands, detect integrations
  trust [--check] [--by]  Approve the config commands block (TOFU); required
                          before gates will execute build/test/lint
  start "<title>"         Create a run, enter PLAN, print the plan playbook.
                          One active run per branch: a branch with a run
                          already in flight is resumed, not restarted
    [--profile <p>]       Phases to run: feature|bugfix|refactor|docs (default feature)
    [--target <a,b>]      Override target resolution (comma-separated names);
                          wins over file-based resolution for this run
  approve [--by] [--reason]  Record PLAN sign-off, bound to the plan's content
  status                  Active run for the current branch, plus any other
                          branches with a run in flight
  check [--run <id>]      Run the current gate; exit code = verdict
  next [--run <id>]       Check + advance on pass; on fail, print what's missing
  review [--fresh] [--by] [--run <id>]  Emit a self-contained review packet
                          (REVIEW phase); --fresh regenerates it from the
                          current code
  retro [--run <id>]      Sync retro.md into the Agnosgram journal (RETRO
                          phase); no-op without a .agnosgram/ store
  report [<run-id>]       Per-run summary: durations, gate failures, findings
                          (falls back to an archived summary after prune)
  prune [--keep n] [--days n] [--dry-run]  Archive non-active runs past the
                          retention window to .gate/archive/, then remove them
  skip <phase> --reason [--by] [--run <id>]  Human-authorized skip of the
                          current phase (recorded with who and why; shown by
                          \`gate report\`)
  log <file> [--run <id>] Register an artifact against the current phase
  playbook [phase] [--run <id>]  Print the active playbook for a phase

Global options:
  --json                  Machine-readable JSON output
  --format json|toon      Choose the serializer (toon: uniform arrays only)
  --run <id>              Act on a specific run instead of resolving the
                          current branch's active run (required on a detached
                          HEAD, where there's no branch to resolve from)
  -h, --help              Show this help
  -v, --version           Show version

The agent loop is two commands: \`gate playbook\` (what do I do?) → \`gate next\`
(am I done?). All state lives on disk under .gate/.`;

type Handler = (args: ReturnType<typeof parseArgs>) => void;

const COMMANDS: Record<string, Handler> = {
  init: cmdInit,
  adapt: cmdAdapt,
  trust: cmdTrust,
  start: cmdStart,
  approve: cmdApprove,
  status: cmdStatus,
  check: cmdCheck,
  next: cmdNext,
  review: cmdReview,
  retro: cmdRetro,
  report: cmdReport,
  prune: cmdPrune,
  skip: cmdSkip,
  log: cmdLog,
  playbook: cmdPlaybook,
};

function main(argv: string[]): void {
  const args = parseArgs(argv);

  if (args.flags.help === true || args.flags.h === true || args.command === "help") {
    process.stdout.write(HELP + "\n");
    return;
  }
  if (args.flags.version === true || args.flags.v === true || args.command === "version") {
    process.stdout.write(readVersion() + "\n");
    return;
  }
  if (!args.command) {
    process.stdout.write(HELP + "\n");
    return;
  }

  const handler = COMMANDS[args.command];
  if (!handler) {
    process.stderr.write(`gate: unknown command "${args.command}"\n\n${HELP}\n`);
    process.exitCode = 2;
    return;
  }

  try {
    handler(args);
  } catch (err) {
    if (err instanceof UsageError) {
      process.stderr.write(`gate: ${err.message}\n`);
      process.exitCode = 2;
    } else if (err instanceof GateError) {
      process.stderr.write(`gate: ${err.message}\n`);
      process.exitCode = err.exitCode;
    } else {
      process.stderr.write(`gate: ${(err as Error).message}\n`);
      process.exitCode = 1;
    }
  }
}

main(process.argv.slice(2));
