# TEST playbook

You are in the **TEST** phase. Prove each acceptance criterion with a real test.
Gate will not let you call this done over a red or hollow suite.

## Do

1. For every criterion with `verify: test: <name>`, write or extend a test whose
   **title contains that substring**. That is the mechanical link the gate checks -
   but it only works if `commands.test` actually emits a JSON test report (see
   "Report" below); with no JSON reporter wired up, `test.criteria` fails closed
   with "no parseable test report found" no matter how well the test matches.
   If this repo's test command has no JSON reporter and you're not going to wire
   one up now, use `verify: manual` in plan.md instead - don't leave `test:`
   criteria that can never pass.
2. **Never weaken an assertion** to make a suite pass. Never delete a failing test
   to make it green. If a test reveals a bug, that is a DEBUG concern (later
   milestone) - do not paper over it here.
3. Run the **full suite**, not just the new tests.
4. **No skipped/pending tests.** If something can't run yet, it belongs in a
   separate run, not skipped here.
5. If a diff-coverage threshold is configured, cover the lines you changed.

## Gate checks

- `test` command exits 0.
- Every `test:`-verified criterion maps to a named **passing** test.
- No skipped tests.
- Diff coverage ≥ threshold (only when a coverage command + threshold are set).

## Report

Evidence must come from the run Gate itself executes - a report staged in
advance (by hand, or via `gate log test-report.json`) is never accepted, even
if it's named `test-report.json`. The real sources, in order:

1. **The test command's own stdout**, captured by Gate as it runs. Point your
   runner's JSON reporter at stdout and Gate parses it directly - no extra
   step needed. Gate persists a normalized copy to `test-report.json` in the
   run folder for the audit trail.
2. **A file the command writes *during this run*** to the path Gate puts in
   the `$GATE_TEST_REPORT` env var (set before the test command runs). A file
   that already existed before the run started doesn't count, even if it has
   the right name - only a file the run itself produced during the run.

Jest/Vitest `--reporter=json` and the generic `{ "tests": [{ "name",
"status" }] }` contract are both understood.

### Runner wiring

- **Vitest / Jest**: `commands.test` in `.gate/config.yml` should run with a
  JSON reporter, e.g. `"vitest run --reporter=json"` or
  `"jest --reporter=json"`. To use the file path instead of stdout:
  `"vitest run --reporter=json --outputFile=$GATE_TEST_REPORT"`.
- **Go**: `go test -json ./...` emits one JSON object per line (`{"Action":
  "pass", "Test": "..."}`, ...), not the `{ "tests": [...] }` shape Gate
  understands. Wrap it: run `go test -json ./...`, translate each test's
  final `pass`/`fail`/`skip` action into `{ "name": "<Test>", "status":
  "passed"|"failed"|"skipped" }`, and either print the translated JSON on
  stdout or write it to `$GATE_TEST_REPORT`.
- **Anything else**: emit the generic `{ "tests": [...] }` contract yourself,
  on stdout or at `$GATE_TEST_REPORT`.

## Advance

`gate next` to enter REVIEW - a fresh reviewer, not you, judges the diff
before RETRO and DONE. `gate status` shows the run's full phase sequence if
you want to confirm.
