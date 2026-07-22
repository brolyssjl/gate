/**
 * Minimal TOON (Token-Oriented Object Notation) encoder.
 *
 * Scope (kickoff decision "uniform arrays only"): the token win comes entirely
 * from rendering uniform arrays of flat objects as a header + comma rows. Other
 * shapes fall back to a readable, indented, YAML-like form. This is an encoder
 * only; Gate never needs to parse TOON back (JSON remains the durable format).
 */

type Json = null | boolean | number | string | Json[] | { [k: string]: Json };

const INDENT = "  ";

export function encodeToon(value: unknown): string {
  const v = value as Json;
  if (isPrimitive(v)) return encodePrimitive(v);
  if (Array.isArray(v)) return encodeArray("", v, 0).trimStart();
  return encodeObject(v as Record<string, Json>, 0);
}

function encodeObject(obj: Record<string, Json>, depth: number): string {
  const pad = INDENT.repeat(depth);
  const lines: string[] = [];
  for (const [key, val] of Object.entries(obj)) {
    if (isPrimitive(val)) {
      lines.push(`${pad}${encodeKey(key)}: ${encodePrimitive(val)}`);
    } else if (Array.isArray(val)) {
      lines.push(encodeArray(key, val, depth));
    } else {
      lines.push(`${pad}${encodeKey(key)}:`);
      lines.push(encodeObject(val as Record<string, Json>, depth + 1));
    }
  }
  return lines.join("\n");
}

function encodeArray(key: string, arr: Json[], depth: number): string {
  const pad = INDENT.repeat(depth);
  const label = key ? `${encodeKey(key)}` : "";
  if (arr.length === 0) return `${pad}${label}[0]:`;

  // Primitive array → inline comma list.
  if (arr.every(isPrimitive)) {
    return `${pad}${label}[${arr.length}]: ${arr.map(encodePrimitive).join(",")}`;
  }

  // Uniform array of flat objects → tabular header + rows (the token win).
  const table = asUniformTable(arr);
  if (table) {
    const header = `${pad}${label}[${arr.length}]{${table.fields.map(encodeKey).join(",")}}:`;
    const rows = table.rows.map(
      (row) => INDENT.repeat(depth + 1) + row.map(encodePrimitive).join(","),
    );
    return [header, ...rows].join("\n");
  }

  // Fallback: list form with one nested item per entry.
  const header = `${pad}${label}[${arr.length}]:`;
  const items = arr.map((item) => {
    const body = isPrimitive(item)
      ? encodePrimitive(item)
      : Array.isArray(item)
        ? encodeArray("", item, depth + 2).trimStart()
        : "\n" + encodeObject(item as Record<string, Json>, depth + 2);
    return `${INDENT.repeat(depth + 1)}- ${body.startsWith("\n") ? body.slice(1) : body}`;
  });
  return [header, ...items].join("\n");
}

function asUniformTable(arr: Json[]): { fields: string[]; rows: Json[][] } | null {
  const first = arr[0];
  if (first === null || typeof first !== "object" || Array.isArray(first)) return null;
  const fields = Object.keys(first);
  if (fields.length === 0) return null;
  const rows: Json[][] = [];
  for (const item of arr) {
    if (item === null || typeof item !== "object" || Array.isArray(item)) return null;
    const obj = item as Record<string, Json>;
    const keys = Object.keys(obj);
    if (keys.length !== fields.length || !fields.every((f) => f in obj)) return null;
    const row: Json[] = [];
    for (const f of fields) {
      const cell = obj[f]!;
      if (!isPrimitive(cell)) return null; // nested values disqualify the table form
      row.push(cell);
    }
    rows.push(row);
  }
  return { fields, rows };
}

function isPrimitive(v: Json): v is null | boolean | number | string {
  return v === null || typeof v !== "object";
}

function encodePrimitive(v: Json): string {
  if (v === null) return "null";
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number") return Number.isFinite(v) ? String(v) : "null";
  if (typeof v !== "string") return "null"; // non-primitive cannot reach here in practice
  return needsQuote(v) ? quote(v) : v;
}

function encodeKey(key: string): string {
  return needsQuote(key) ? quote(key) : key;
}

function needsQuote(s: string): boolean {
  return (
    s.length === 0 ||
    /[,:{}[\]"\n]/.test(s) ||
    s !== s.trim() ||
    s === "null" ||
    s === "true" ||
    s === "false" ||
    /^-?\d/.test(s)
  );
}

function quote(s: string): string {
  return '"' + s.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n") + '"';
}
