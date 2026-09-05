# DEBUG playbook

You are in the **DEBUG** phase. A bug is a claim about reality you don't yet
understand. Gate will not let you call it fixed until you've *reproduced* it,
diagnosed it with an explicit hypothesis loop, and left the triggering test green
with no regressions. Log every cycle in `debug-log.md` in the run folder.

## The protocol (one cycle at a time)

1. **Reproduce** - get the failure to happen on demand, ideally as a failing
   test. Name that test's title (or a substring) in `triggering_test`. Set
   `reproduced: true` only once you've actually seen it fail. If you can't
   reproduce it, you cannot fix it - keep at the reproduce step.
2. **Hypothesize** - write down *one* concrete, falsifiable cause. Not "something
   with state"; rather "the cache isn't invalidated when X changes."
3. **Predict** - state what you'd observe if the hypothesis were true (and,
   ideally, what you'd observe if it were false).
4. **Experiment** - run the smallest test that discriminates: a log line, a
   breakpoint, a one-line change. Record what you actually did.
5. **Observe & conclude** - write down what you saw and what it means: hypothesis
   confirmed, refuted, or refined. If refuted, start a new cycle. Mark a finished
   cycle `status: complete`.

Do not skip straight to a fix. A fix that lands without a confirmed hypothesis is
a guess, and guesses regress.

## debug-log.md schema

```markdown
---
triggering_test: "resets password"   # substring of the failing test's title
reproduced: true
cycles:
  - hypothesis: "Token TTL is compared in ms against a seconds value"
    prediction: "Logging both sides shows a 1000x mismatch"
    experiment: "Added a temporary log of expiry and now()"
    observation: "expiry=1699999999000, now=1699999999 - off by 1000x"
    conclusion: "Confirmed: units mismatch in isExpired()"
    status: complete
---

# Debug log: <title>

Free-form narrative, links, stack traces.
```

## Gate checks

- `debug-log.md` exists and matches the schema.
- `reproduced: true` (your attestation - the test below is the mechanical proof).
- At least one **complete** cycle (all five fields filled, `status: complete`).
- Every touched file is declared in `plan.md` (scope discipline still applies).
- The **triggering test passes** and the whole suite is green (no regressions).
  The test must appear by name in the run's report: emit JSON on stdout or have
  the runner write to `$GATE_TEST_REPORT`. No parseable report fails the gate -
  exit 0 alone cannot name your fix.

## Advance

`gate next`. Fix the root cause you confirmed - not the symptom.
