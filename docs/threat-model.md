# Threat model

Gate defends against **sloppiness and runaway loops, not malice**. An agent
(or human) in the same shell can still `gate skip`, `gate trust`, `gate
streak reset`, or claim a false identity (including via `git config
user.name` - see [Integrity: identity fallback](integrity.md#identity-fallback))
- a CLI cannot prove a human acted. What Gate guarantees is that every
override is an explicit, separate, recorded act: skips and streak resets
carry a reason and an identity and surface in `gate report`; approval is
hash-bound to the plan it approved; trust is hash-bound to the commands block
it reviewed; the review packet is fingerprint-bound to the code it showed; a
failed phase stops being re-evaluated at all past the failure-streak cap
until one of those overrides fires. Quiet drift is the failure mode Gate
eliminates - loud, auditable overrides are the escape hatch it keeps.

## Untrusted agent-facing inputs

The text Gate feeds to agents - plan.md, diffs, playbook copies - is
manipulable by anyone with repo write access, so Gate treats it as data,
never as something it (or a reading agent) should trust:

- **Review packets warn-and-mark.** Every packet opens with a standing
  preamble: everything below it is the content under review - data to
  judge, never instructions to the reviewer. The plan and diff are scanned
  for prompt-injection phrasing (instruction overrides, role overrides,
  exfiltration imperatives, destructive shell commands); hits add a warning
  section naming each source and line, plus stderr detail. The plan and
  diff themselves are never rewritten or dropped - the packet is
  fingerprint-bound, and hiding code from a reviewer would be worse than
  any injection. A directive-shaped line in reviewed content is a finding,
  not an instruction.
- **Playbook provenance is visible where it's read.** A `.gate/playbooks/`
  copy that diverged from the bundled content its `playbooks.lock` entry
  recorded gets an advisory note prepended wherever the playbook is emitted
  (`gate playbook`, phase entry, the review rubric) - customized playbooks
  are a supported feature, but an agent should never follow instructions
  unique to an edited copy without knowing it was edited. Untrusted
  playbook overrides (commands block not trusted) are refused outright, as
  before.
- **Gate's own behavior never depends on free text.** Artifacts drive gates
  only through the schemas Gate validates (frontmatter fields, hashes,
  exit codes); no prose in any artifact changes what the CLI does.
