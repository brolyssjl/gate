import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { ADAPTER_KEYS, ADAPTERS, buildPointerBody, type Adapter } from "../adapters/index.js";
import { upsertManagedBlock } from "../core/markers.js";
import { emit, requireRoot, UsageError, type ParsedArgs } from "./shared.js";

export type AdaptAction = "created" | "updated" | "unchanged";

export interface AdaptResult {
  adapter: string;
  path: string;
  action: AdaptAction;
}

/**
 * Inject or refresh one adapter's managed block. Idempotent: a second call
 * with nothing changed reports "unchanged" and does not touch the file.
 */
export function applyAdapter(root: string, adapter: Adapter): AdaptResult {
  const target = join(root, adapter.targetPath);
  const body = buildPointerBody();

  let existing = "";
  if (existsSync(target)) {
    existing = readFileSync(target, "utf8");
  } else if (adapter.dedicatedFile && adapter.preamble) {
    existing = adapter.preamble;
  }

  const next = upsertManagedBlock(existing, body);
  const existedBefore = existsSync(target);

  if (existedBefore && next === existing) {
    return { adapter: adapter.key, path: adapter.targetPath, action: "unchanged" };
  }

  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, next);
  return { adapter: adapter.key, path: adapter.targetPath, action: existedBefore ? "updated" : "created" };
}

/**
 * `gate adapt [adapter...]` - write (or refresh) agent config pointer blocks.
 * With no arguments, writes every known adapter (all are cheap, idempotent
 * managed-block writes, so there's no harm doing all of them by default).
 * Named adapters restrict the run to just those.
 */
export function cmdAdapt(args: ParsedArgs): void {
  const root = requireRoot();
  const requested = args.positionals;

  const unknown = requested.filter((t) => !ADAPTERS[t]);
  if (unknown.length > 0) {
    throw new UsageError(`unknown adapter(s): ${unknown.join(", ")} (known: ${ADAPTER_KEYS.join(", ")})`);
  }

  const targets = requested.length > 0 ? requested : ADAPTER_KEYS;
  const results = targets.map((key) => applyAdapter(root, ADAPTERS[key]!));

  const human = results
    .map((r) => `  ${r.action.padEnd(9)} ${r.path}  (${ADAPTERS[r.adapter]!.name})`)
    .join("\n");

  emit(human, { adapters: results }, args.flags);
}
