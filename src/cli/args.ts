export interface ParsedArgs {
  command: string | null;
  positionals: string[];
  flags: Record<string, string | boolean>;
}

/**
 * Minimal argv parser: `--flag value`, `--flag=value`, boolean `--flag`, and
 * positionals. Kept dependency-free so `gate status`/`gate next` stay fast on
 * the agent hot path.
 */
export function parseArgs(argv: string[]): ParsedArgs {
  const rest = [...argv];
  // The command is the first token only when it isn't a flag; `gate --version`
  // and `gate -h` have no command, just global flags.
  const command = rest[0] !== undefined && !rest[0].startsWith("-") ? rest.shift()! : null;
  const positionals: string[] = [];
  const flags: Record<string, string | boolean> = {};

  for (let i = 0; i < rest.length; i++) {
    const arg = rest[i]!;
    if (arg.startsWith("--")) {
      const body = arg.slice(2);
      const eq = body.indexOf("=");
      if (eq >= 0) {
        flags[body.slice(0, eq)] = body.slice(eq + 1);
      } else {
        const next = rest[i + 1];
        if (next !== undefined && !next.startsWith("--")) {
          flags[body] = next;
          i++;
        } else {
          flags[body] = true;
        }
      }
    } else if (arg.startsWith("-") && arg.length > 1) {
      // Short boolean flag, e.g. -v / -h.
      flags[arg.slice(1)] = true;
    } else {
      positionals.push(arg);
    }
  }
  return { command: command === null ? null : command, positionals, flags };
}
