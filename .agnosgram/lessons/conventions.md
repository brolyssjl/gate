# Conventions - "always do Y"

_Patterns that worked, worth repeating. Same frontmatter schema as pitfalls._

---
id: CON-001
type: convention
scope: [run-json, serialization]
confidence: high
created: 2026-09-05
last_verified: 2026-09-05
source: journal/2026-09.md
---
Additive run.json keys follow the targetOverride precedent: omit the key
entirely when absent (never null), no schema bump, and the byte-for-byte
serialization test stays green. Reserve schema bumps for real migrations.

---
id: CON-002
type: convention
scope: [testing, gates]
confidence: high
created: 2026-09-05
last_verified: 2026-09-05
source: journal/2026-09.md
---
This repo's gate test command is scripts/test-with-report.sh, which bridges
cargo test output to the generic $GATE_TEST_REPORT contract - so plan
criteria use verify: "test: <substring>" and get verified by name; never
fall back to verify: manual for behavior a test can prove.
