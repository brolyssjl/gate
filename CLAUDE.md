<!-- agnosgram:start -->
<!-- Managed by agnosgram. Edits inside this block are overwritten on `agnosgram adapt`. -->
## Project memory (Agnosgram)

This project keeps durable, agent-agnostic memory in `.agnosgram/` (plain
Markdown, reviewed in PRs). **Before doing any work:**

1. Read `.agnosgram/MEMORY.md` and follow its reading protocol.
2. Always load `.agnosgram/state/status.md` and `.agnosgram/lessons/*`.
3. Read `.agnosgram/context/*` and `.agnosgram/decisions/` only for the
   areas you are about to touch.
4. Before ending the session, record what happened with `agnosgram log`.
<!-- agnosgram:end -->

<!-- gate:start -->
<!-- Managed by gate. Edits inside this block are overwritten on `gate adapt`. -->
## Gate quality flow

This repo uses Gate to enforce PLAN -> IMPLEMENT -> TEST -> REVIEW -> RETRO
as a state machine with deterministic gates. Before doing any work:

1. Run `gate status`. No active run? Start one: `gate start "<title>"`.
2. Run `gate playbook` for the current phase - that is your instruction set.
3. Do the work the playbook describes.
4. Run `gate next`. It checks the gate and advances on pass; on fail it
   prints exactly what evidence is missing - fix that, don't argue with it.
5. Loop 2-4 until the run reaches DONE.

Never hand-edit files under `.gate/runs/` (use `gate log` to register
artifacts). A red gate means missing evidence, not a suggestion to skip it.
<!-- gate:end -->
