# Domain

_Business/domain glossary and rules an agent won't infer from the code._

- **Run:** one unit of gated work, keyed by branch (one active run per
  branch), state in `.gate/runs/<id>/run.json`
- **Phase / profile:** runs walk a profile's phase sequence (feature:
  PLAN -> IMPLEMENT -> TEST -> REVIEW -> RETRO -> DONE; also bugfix,
  refactor, docs); each gated phase has a deterministic gate + a playbook
- **Gate:** the deterministic check for a phase; exit 0/1, or 3 once the
  failure-streak cap blocks the phase
- **Trust (TOFU):** commands in `.gate/config.yml` execute only after
  `gate trust` hashes the block; any config edit invalidates the hash
- **Override:** the explicit, audited ways past a gate - `skip`,
  `streak reset` - always recorded with reason + identity
- **Threat model:** gate defends against sloppiness and runaway loops,
  not malice; identity is deliberately weak/spoofable, but every override
  is a loud, recorded act
- **Review packet:** self-contained review input, fingerprint-bound to
  the tree; reviewer independence is advisory until real session identity
  exists (roadmap "Reviewer identity threading")
