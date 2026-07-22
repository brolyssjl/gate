import { serialize, isFormat, type Format } from "../serialize/index.js";
import type { ParsedArgs } from "./args.js";

/** Resolve the output format from `--format <fmt>` / `--json` flags. */
export function resolveFormat(flags: ParsedArgs["flags"]): Format | "human" {
  const fmt = flags.format;
  if (typeof fmt === "string") {
    if (!isFormat(fmt)) throw new UsageError(`unknown --format "${fmt}" (use json or toon)`);
    return fmt;
  }
  if (flags.json === true) return "json";
  return "human";
}

/**
 * Emit a command result. When a structured format is requested, print the data;
 * otherwise print the human string. This keeps `--json` output schema-stable for
 * CI and agents while humans get readable text by default.
 */
export function emit(human: string, data: unknown, flags: ParsedArgs["flags"]): void {
  const fmt = resolveFormat(flags);
  if (fmt === "human") process.stdout.write(human.endsWith("\n") ? human : human + "\n");
  else process.stdout.write(serialize(data, fmt) + "\n");
}

/** Thrown for bad CLI usage; the entrypoint prints it without a stack trace. */
export class UsageError extends Error {}

/** Thrown for expected failures (no run, gate red); entrypoint sets exit code. */
export class GateError extends Error {
  constructor(
    message: string,
    readonly exitCode = 1,
  ) {
    super(message);
  }
}
