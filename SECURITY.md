# Security policy

## Supported versions

Only the latest `1.x` release is supported. Security fixes land on `main`
and ship in the next tagged release; there are no backported patch releases
for older tags.

## Reporting a vulnerability

Report privately through GitHub's private vulnerability reporting on this
repository's **Security** tab (Security -> Report a vulnerability). Do not
open a public issue for a suspected vulnerability.

You should get an acknowledgement within 7 days. From there we'll work with
you on a fix and a disclosure timeline before anything is made public.

## Scope

Gate's [threat model](docs/threat-model.md) spells this out in full; the
short version:

- Gate defends against sloppiness and runaway agent loops, not a malicious
  actor with shell access next to it - anyone who can run commands in the
  same shell can already `gate skip`, `gate trust`, or claim a false
  identity. What's in scope is anything that breaks Gate's actual
  guarantees: hash-bound approval, hash-bound trust, fingerprint-bound
  review packets, and recorded (not silent) overrides.
- Text Gate feeds to agents (plan.md, diffs, playbook copies) is untrusted
  input from anyone with repo write access. A prompt-injection payload that
  Gate fails to warn-and-mark, or that changes Gate's own behavior rather
  than just being displayed, is in scope.
- The install and release pipeline (`install.sh`, `.github/workflows/*`) is
  in scope: checksum or provenance bypass, unpinned actions, or shell
  injection via untrusted input.
- Repository settings (branch/tag rulesets, secret scanning) are owner
  configuration, not code, but a gap there that undermines the above is
  still worth reporting.

## Verifying a release

`install.sh` verifies every downloaded binary against the release's
`SHA256SUMS`, then (when `gh` is installed) against its build-provenance
attestation via `gh attestation verify` - see
[CONTRIBUTING.md](CONTRIBUTING.md#release-provenance) for how to run either
check by hand.
