# Configuration, adapters, and upkeep

## Configuration

`.gate/config.yml` (inferred by `gate init`, then hand-editable):

```yaml
commands:
  build: "npm run build"
  test:  "npm test"
  lint:  "npm run lint"
  coverage: "npm run coverage"   # optional
thresholds:
  diff_coverage: 80              # only enforced when a coverage command is set
  failure_streak_limit: 3        # optional - see loop enforcement in integrity.md; 0 disables it
phases:
  plan: required
  implement: required
  test: required
integrations:
  agnosgram: auto                # advisory only - presence changes hint lines and check behavior
  sdd: auto
targets: {}                      # optional - see docs/runs-and-concurrency.md
retention: {}                    # optional - gate prune defaults, e.g. { keep: 10, days: 30 }
scope_ignore: []                 # optional - see docs/runs-and-concurrency.md; seeded by gate init
identity:
  require_identity: false        # optional - see docs/integrity.md
```

### Playbooks

A target's `playbooks:` overlay path (see [Targets](runs-and-concurrency.md#targets-multi-stack-repos))
must be **relative to the project root**: no absolute path, no `..`
component, and it must resolve (after canonicalizing symlinks) to a regular
file under the root - not a directory, FIFO, or device. `gate` refuses a
config that violates this, naming the offending key and path, before it
will run anything; a path that simply doesn't exist yet is not a violation.

`.gate/playbooks/<phase>.md` overrides carry a provenance note wherever
they're read (`gate playbook`, phase entry, the review rubric): a copy with
no entry in `.gate/playbooks.lock` prints a loud **UNVERIFIED PLAYBOOK**
note (it was never materialized by `gate init`/`update`), a copy whose
content diverged from its lock entry prints a quieter advisory note, and a
pristine copy prints nothing. See [Integrity: command trust
(TOFU)](integrity.md#command-trust-tofu) for the full picture, including how
trust and provenance interact.

Both playbook overrides and target overlays are scanned for
prompt-injection phrasing, same as the plan and diff (see [Threat
model](threat-model.md)) - a hit is flagged inline, never dropped.

Diff coverage understands five report formats, auto-detected under `coverage/`
(point your configured `coverage` command at the matching path), or pinned
explicitly with `coverage_format`:

- `istanbul` - `coverage/coverage-final.json` (jest, vitest --coverage)
- `generic` - `coverage/gate-coverage.json`:
  `{ "files": [{ "path", "covered": [], "uncovered": [] }] }`
- `coverage-py` - `coverage/coverage.json` (`coverage json -o coverage/coverage.json`)
- `go-cover` - `coverage/go-cover.out` (`go test -coverprofile=coverage/go-cover.out`);
  profile paths are de-prefixed against `go.mod`'s `module` line so they line
  up with git's repo-relative paths
- `lcov` - `coverage/lcov.info` (nyc, gcov, and many others' standard output path)

`coverage_format` can also be set per target (see docs/runs-and-concurrency.md),
overriding the top-level default for that stack only - useful in a
multi-language repo where one target's tests emit lcov and another's emit
coverage.py.

## Output formats

Human-readable by default. `--json` for CI and agents (stable schema). `--format
toon` opts into [TOON](https://github.com/toon-format) for uniform, tabular
payloads (findings lists) where it saves tokens; JSON stays the default because
TOON is worse on small objects.

## Agent adapters

`gate adapt [adapter...]` writes (or refreshes) a pointer block into each
agent's config file - the same ~12-line loop (status → playbook → work →
next), composed with the detected SDD when one is present (see
[SDD composition](sdd-composition.md)). With no arguments it writes every
adapter; idempotent - running it again with nothing changed reports
`unchanged` and doesn't touch the file. `gate init` installs `claude` +
`agents` by default; `gate adapt` is how you add the rest, any time.

| Adapter | Target file |
|---|---|
| `claude` | `CLAUDE.md` |
| `claude-skill` | `.claude/skills/gate/SKILL.md` |
| `cursor` | `.cursor/rules/gate.mdc` |
| `cline` | `.clinerules/gate.md` |
| `windsurf` | `.windsurf/rules/gate.md` |
| `agents` | `AGENTS.md` |

These are ergonomics only - the contract is always the CLI; every adapter
just tells the agent to run it.

## `gate doctor` and `gate update`

An installed-but-untended gate leaves no trace an agent will find: adapter
blocks go stale as `gate` itself is upgraded, playbook copies under
`.gate/playbooks/` drift from the bundled defaults, and a repo with an SDD
now present might still be carrying the pre-composition generic pointer
body. `gate doctor` diagnoses all of it, read-only:

```bash
gate doctor          # human-readable findings, [info] vs [action] severity
gate doctor --json    # same findings, structured
```

It checks: which agent files are missing gate's managed block or carry a
stale one; whether each playbook copy under `.gate/playbooks/` is current,
pristine-but-outdated (bundled content moved on, safe to auto-refresh),
user-edited (never touched without asking), or of unknown provenance (no
manifest entry - a pre-manifest project, or a hand-deleted one); trust
status; and whether `.gate/config.yml` exists and parses. Exit code 0 when
nothing actionable was found, 1 otherwise - script against it like `gate
trust --check`.

`gate update` applies what `doctor` diagnosed - the upgrade story:

```bash
gate update                       # refresh what doctor flagged; never touches a user-edited playbook
gate update --adapt cursor,cline  # also install adapter targets not covered by default
gate update --force-playbooks     # replace user-edited/unknown-provenance playbooks too
```

Managed adapter blocks are gate-owned by contract, so they're always safe to
regenerate. A playbook a human edited is never silently replaced -
`--force-playbooks` is the explicit override, and forcing one is reported
alongside a reminder to re-run `gate trust` (playbook overrides ride in the
same trust hash as `commands:` - see
[Integrity: command trust (TOFU)](integrity.md#command-trust-tofu)). Every
copy `gate init`/`gate update` materializes gets a version-stamped manifest
entry (`.gate/playbooks.lock`) so a later `doctor`/`update` - on this machine
or a teammate's - can tell "unchanged since materialized" apart from
"hand-edited"; like `config.yml`, it stays tracked so CI and every clone see
the same picture. Trust (`gate trust`) is the one exception - it lives in a
machine-local store outside the repo (see
[Integrity: command trust (TOFU)](integrity.md#command-trust-tofu)), not in
`.gate/playbooks.lock` or anywhere else tracked, so it is never part of this
picture. Both commands are idempotent: run either twice right after itself
and the second run reports nothing to do.

## `gate guard` (opt-in pre-commit hook)

Off by default; only the explicit `gate guard install` writes anything.
It manages `.git/hooks/pre-commit`, backing up and chaining any hook already
there (or refusing, with instructions, if it can't compose safely):

```bash
gate guard install      # writes .git/hooks/pre-commit (backs up + chains an existing one)
gate guard uninstall    # removes it, restoring the backup if there was one
```

The hook runs only cheap, deterministic checks - never a build/test command:
an active run exists for the branch, staged files stay within the plan's
declared scope, and the run isn't still in PLAN (nothing should be committed
before IMPLEMENT starts). It is **advisory, never load-bearing** - the real
enforcement is `gate check`/`gate next`. Escape hatches always work: `git
commit --no-verify` skips it for one commit; `GATE_GUARD=0 git commit`
disables it globally without uninstalling. `init` never touches git hooks.
