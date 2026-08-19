# Decision records

This directory holds short, dated records of decisions that would otherwise
only live in a PR description or a roadmap footnote. They exist so a later
reader (human or agent) can find out *why* something is the way it is without
archaeology through git log.

## Format

One file per decision: `NNNN-short-title.md`, numbered sequentially, never
renumbered or reused even if a decision is later superseded. Each record is
short - a paragraph or two, not a design doc - and states:

- **Status** - proposed, accepted, or superseded (and by what, if so)
- **Date** - when the decision was made
- **Context** - the situation that forced a decision
- **Decision** - what was decided, stated plainly
- **Consequences** - what this binds or rules out going forward

A decision record is never edited to reflect new information after the fact;
if circumstances change, add a new record and mark the old one superseded.
This mirrors common lightweight ADR (Architecture Decision Record) practice,
trimmed to what this project actually needs.
