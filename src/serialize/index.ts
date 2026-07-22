import { encodeToon } from "./toon.js";

export type Format = "json" | "toon";

export function isFormat(value: string): value is Format {
  return value === "json" || value === "toon";
}

/**
 * Render agent-facing structured output. JSON is the default and the only
 * format CI should rely on. TOON is opt-in for token savings on uniform,
 * tabular payloads (findings/lessons lists) and worse on small objects, so it
 * is never the default (kickoff decision).
 */
export function serialize(value: unknown, format: Format): string {
  return format === "toon" ? encodeToon(value) : JSON.stringify(value, null, 2);
}
