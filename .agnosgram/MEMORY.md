# Project memory - read this first

This is Agnosgram, an agent-agnostic memory store. It is plain Markdown, lives in
the repo, and is reviewed in PRs like any other code.

## Protocol for agents
1. Read `state/status.md` (always) - current focus and in-flight work.
2. Read `lessons/pitfalls.md` and `lessons/conventions.md` (always).
3. Read `context/*` only for areas you will touch.
4. Read `decisions/` only when about to change something architectural -
   check for an existing decision before proposing a change to settled matters.
5. Before ending a session, append a journal entry with `agnosgram log`
   (or by hand into `journal/2026-09.md`).

Specs say what the system should be; Agnosgram says what we learned making it.
A fact belongs in exactly one place; the other side links to it.

## Freshness
| File | Last verified | Budget |
|------|---------------|--------|
| state/status.md | 2026-09-05 | 400 tokens |
| context/architecture.md | 2026-09-05 | 1500 tokens |
| context/stack.md | 2026-09-05 | 800 tokens |
| context/domain.md | 2026-09-05 | 1000 tokens |
| lessons/pitfalls.md | 2026-09-05 | 1000 tokens |
| lessons/conventions.md | 2026-09-05 | 1000 tokens |
