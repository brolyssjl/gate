#!/usr/bin/env node
import { parseArgs } from "./cli/args.js";
import { GateError, UsageError } from "./cli/output.js";
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

const VERSION = "0.0.0";

const HELP = `gate — an agent-agnostic quality harness (umpire, not a driver).

Usage: gate <command> [options]

Commands:
  init [--refresh]        Scaffold .gate/, infer commands, detect integrations
  trust [--check] [--by]  Approve the config commands block (TOFU); required
                          before gates will execute build/test/lint
  start "<title>"         Create a run, enter PLAN, print the plan playbook
  approve [--by] [--reason]  Record PLAN sign-off, bound to the plan's content
  status                  Current run, phase, what the gate waits for
  check                   Run the current gate; exit code = verdict
  next                    Check + advance on pass; on fail, print what's missing
  skip <phase> --reason   Human-authorized skip of the current phase (audited)
  log <file>              Register an artifact against the current phase
  playbook [phase]        Print the active playbook for a phase

Global options:
  --json                  Machine-readable JSON output
  --format json|toon      Choose the serializer (toon: uniform arrays only)
  -h, --help              Show this help
  -v, --version           Show version

The agent loop is two commands: \`gate playbook\` (what do I do?) → \`gate next\`
(am I done?). All state lives on disk under .gate/.`;

type Handler = (args: ReturnType<typeof parseArgs>) => void;

const COMMANDS: Record<string, Handler> = {
  init: cmdInit,
  trust: cmdTrust,
  start: cmdStart,
  approve: cmdApprove,
  status: cmdStatus,
  check: cmdCheck,
  next: cmdNext,
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
    process.stdout.write(VERSION + "\n");
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
