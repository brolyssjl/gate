export interface NormalizedTest {
  name: string;
  status: "passed" | "failed" | "skipped";
}

export interface NormalizedReport {
  tests: NormalizedTest[];
  skipped: number;
}

/**
 * Normalize a test report from either the jest/vitest JSON shape or the generic
 * contract `{ tests: [{ name, status }] }`. Returns null if unparseable.
 */
export function parseTestReport(raw: string): NormalizedReport | null {
  const data = extractJson(raw);
  if (!data || typeof data !== "object") return null;
  const obj = data as Record<string, unknown>;

  // jest / vitest --reporter=json
  if (Array.isArray(obj.testResults)) {
    const tests: NormalizedTest[] = [];
    for (const suite of obj.testResults as Array<Record<string, unknown>>) {
      const assertions = Array.isArray(suite.assertionResults) ? suite.assertionResults : [];
      for (const a of assertions as Array<Record<string, unknown>>) {
        const name =
          typeof a.fullName === "string" && a.fullName.trim()
            ? a.fullName
            : typeof a.title === "string"
              ? a.title
              : "";
        tests.push({ name, status: mapStatus(a.status) });
      }
    }
    return { tests, skipped: tests.filter((t) => t.status === "skipped").length };
  }

  // generic contract
  if (Array.isArray(obj.tests)) {
    const tests: NormalizedTest[] = [];
    for (const t of obj.tests as Array<Record<string, unknown>>) {
      if (!t || typeof t.name !== "string") continue;
      tests.push({ name: t.name, status: mapStatus(t.status) });
    }
    return { tests, skipped: tests.filter((t) => t.status === "skipped").length };
  }

  return null;
}

/**
 * Extract a JSON object/array from possibly-noisy output. Test runners invoked
 * through `npm test` prefix the report with banner lines (`> pkg@1 test`), so a
 * plain JSON.parse of stdout fails. Try the whole string, then the outermost
 * brace/bracket slice, then each line.
 */
function extractJson(raw: string): unknown {
  const t = raw.trim();
  const attempts: string[] = [t];
  const objStart = t.indexOf("{");
  const objEnd = t.lastIndexOf("}");
  if (objStart >= 0 && objEnd > objStart) attempts.push(t.slice(objStart, objEnd + 1));
  const arrStart = t.indexOf("[");
  const arrEnd = t.lastIndexOf("]");
  if (arrStart >= 0 && arrEnd > arrStart) attempts.push(t.slice(arrStart, arrEnd + 1));
  for (const line of t.split("\n")) {
    const l = line.trim();
    if (l.startsWith("{") || l.startsWith("[")) attempts.push(l);
  }
  for (const candidate of attempts) {
    try {
      return JSON.parse(candidate);
    } catch {
      // try the next candidate
    }
  }
  return null;
}

function mapStatus(s: unknown): NormalizedTest["status"] {
  if (s === "passed" || s === "failed") return s;
  if (s === "pending" || s === "skipped" || s === "todo" || s === "disabled") return "skipped";
  return "failed";
}
