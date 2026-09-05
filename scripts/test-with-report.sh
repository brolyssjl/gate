#!/bin/bash
# Gate test command: run the full cargo suite and, when Gate provides
# $GATE_TEST_REPORT, also emit the generic {"tests":[{name,status}]}
# report it parses for the TEST gate's criterion->test mapping (cargo has
# no stable JSON reporter, so this bridges from the human output). The
# cargo exit code is preserved either way.
#
# Known limits: needs python3 on PATH (absent, the suite still runs and
# gates on its exit code, but no report is written so test.criteria
# reports "no parseable test report"), and the line regex matches unit/
# integration test output only, not doctests (this crate has none).
set -o pipefail

out=$(mktemp)
cargo test --manifest-path rust/Cargo.toml 2>&1 | tee "$out"
code=$?

if [ -n "$GATE_TEST_REPORT" ]; then
  python3 - "$out" "$GATE_TEST_REPORT" <<'EOF'
import json, re, sys
lines = open(sys.argv[1], errors="replace").read().splitlines()
tests = []
for line in lines:
    m = re.match(r"^test ([A-Za-z0-9_:]+) \.\.\. (ok|FAILED|ignored)$", line)
    if m:
        status = {"ok": "passed", "FAILED": "failed", "ignored": "skipped"}[m.group(2)]
        tests.append({"name": m.group(1), "status": status})
json.dump({"tests": tests}, open(sys.argv[2], "w"), indent=1)
EOF
fi

rm -f "$out"
exit $code
