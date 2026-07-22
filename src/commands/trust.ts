import { currentCommandsHash, hasNoCommands, isCommandsTrusted, readTrust, writeTrust } from "../core/trust.js";
import { emit, requireRoot, type ParsedArgs } from "./shared.js";

/**
 * `gate trust` — approve the current `commands:` block (TOFU). Writes the hash
 * to `.gate/trust.json`; the IMPLEMENT/TEST gates refuse to execute commands
 * until this matches. `--check` reports status without writing (exit 0/1).
 */
export function cmdTrust(args: ParsedArgs): void {
  const root = requireRoot();

  if (args.flags.check === true) {
    const trusted = isCommandsTrusted(root);
    const record = readTrust(root);
    const human = trusted
      ? `trusted (${hasNoCommands(root) ? "no commands to run" : currentCommandsHash(root)})`
      : `NOT trusted — run \`gate trust\` (current ${currentCommandsHash(root)}, stored ${record?.commandsHash ?? "none"})`;
    emit(human, { trusted, currentHash: currentCommandsHash(root), storedHash: record?.commandsHash ?? null }, args.flags);
    process.exitCode = trusted ? 0 : 1;
    return;
  }

  const by =
    (typeof args.flags.by === "string" ? args.flags.by : undefined) ??
    process.env.GATE_SESSION_ID ??
    null;
  const record = writeTrust(root, by);
  emit(
    `Trusted the commands block: ${record.commandsHash}` + (by ? ` (by ${by})` : ""),
    { trusted: true, ...record },
    args.flags,
  );
}
