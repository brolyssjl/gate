# Pitfalls - "do not do X"

_Distilled failures. Each entry carries frontmatter (see below) so `doctor` can
track staleness. Add via `distill`; edit by hand when you learn something now._

---
id: LES-001
type: pitfall
scope: [identity, gates]
confidence: high
created: 2026-09-05
last_verified: 2026-09-05
source: journal/2026-09.md
---
Never route a fallback-derived identity into a field a gate compares against.
sessionId feeds reviewer_check's hard-fail (reviewer == implementer), so
letting the git-user.name fallback leak into it would have flipped the
documented solo review flow from advisory to blocking in every one-person
repo. When an identity field gains a weaker source, split audit identity
(startedBy) from compared identity (sessionId) instead (#33).
