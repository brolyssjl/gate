# RETRO playbook

You are in the **RETRO** phase, the last stop before DONE. Three questions,
answered honestly and briefly - this is capture, not essay writing. Gate will
not let you reach DONE on an untouched scaffold.

## Do

1. **What broke?** Anything that failed, surprised you, or cost time -
   a wrong assumption, a flaky test, a misread requirement.
2. **What to avoid next time?** A concrete "don't do X" for a future run.
3. **What convention emerged?** A pattern worth repeating, if one did.

Answer at least one. Empty slots are fine; an entirely empty retro is not -
the gate requires ≥1 entry across the three.

4. If this repo has an `.agnosgram/` project-memory store, run `gate retro` to
   sync your answers into its journal (source-linked to this run). Without a
   store, this step is a no-op - the gate does not require it.

## retro.md schema

```markdown
---
broke:
  - "Assumed the API was idempotent; it wasn't, and a retry double-charged in dev"
avoid:
  - "Don't skip the reproduce step even when the fix looks obvious"
conventions:
  - "Always add a `--dry-run` flag to destructive commands before shipping them"
---

# Retro: <title>

Free-form narrative, if useful.
```

## Gate checks

- `retro.md` exists and matches the schema.
- At least one of `broke`/`avoid`/`conventions` has an entry.
- If `.agnosgram/` is present (and the integration isn't `off`): the entry was
  synced to the journal via `gate retro`. No store detected → this check
  passes with a note; nothing to sync to.

## Advance

`gate retro` (if `.agnosgram/` is present) → `gate next` → DONE.
