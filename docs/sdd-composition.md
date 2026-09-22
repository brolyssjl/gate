# SDD composition

Gate is the agnostic loop enforcer; a Spec-Driven-Development framework
(SDD) supplies the *content* of some phases. With an SDD directory detected
(`openspec/`, `.specify/`, `_bmad/`/`.bmad/`) and `integrations.sdd` not
`off`, Gate composes instead of competing: it maps its own phases against
that framework's own steps - which SDD step **fulfills** a gate phase, which
phases are **gate-only** (no SDD equivalent - TEST/REVIEW/RETRO, typically),
and which SDD step **closes the loop** after DONE. For openspec:

| Gate phase | Fulfilled by |
|---|---|
| PLAN | openspec's `propose` step; approval is always `gate approve`, not openspec's own sign-off |
| IMPLEMENT | openspec's `apply` step |
| TEST / REVIEW / RETRO | gate-only - openspec has no equivalent; the playbook says what and how |
| *(after DONE)* | `openspec archive <change>` closes openspec's own record - Gate doesn't run it |

This composed story shows up everywhere an agent looks: the adapter pointer
blocks (`gate adapt`/`init`), `gate playbook`'s per-phase hint line, and a
one-line reminder printed when a run reaches DONE. Sensible built-in
mappings exist for spec-kit and BMAD too. The PLAN gate's own check is the
read side of the same integration: it requires plan.md's `spec:` field to
cite a path that exists under the detected SDD directory instead of
restating the spec. No SDD directory present, or `integrations.sdd: off`:
none of this runs - the generic pointer body and no spec-citation
requirement, same as before this existed.

A framework Gate doesn't recognize by name (or a built-in mapping you want
to correct) can still declare its own equivalences:

```yaml
integrations:
  sdd_mapping:
    plan: draft            # SDD step that fulfills PLAN
    implement: build        # SDD step that fulfills IMPLEMENT
    closing_step: close     # SDD step that closes the loop after DONE
    closing_command: "myddl close <change>"
```

Every key is optional and independently overridable; an empty string
(`test: ""`) explicitly marks a phase gate-only rather than leaving a
built-in mapping's default in place. This is advisory content only (it
never changes what a gate checks) - deliberately outside the trust hash.
