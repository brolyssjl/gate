# TEST playbook

You are in the **TEST** phase. Prove each acceptance criterion with a real test.
Gate will not let you call this done over a red or hollow suite.

## Do

1. For every criterion with `verify: test: <name>`, write or extend a test whose
   **title contains that substring**. That is the mechanical link the gate checks.
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

The gate reads a test report from `test-report.json` in the run folder if you
register one (`gate log test-report.json`), otherwise it parses the test
command's JSON stdout. Jest/Vitest `--reporter=json` and the generic
`{ "tests": [{ "name", "status" }] }` contract are both understood.

## Advance

`gate next`. In this milestone TEST is the last gate before DONE.
