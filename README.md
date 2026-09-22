<div align="center">
  <img src="docs/assets/gate-mascot.svg" width="150" alt="Gate's mascot: a traffic-officer owl with aviators and a stop paddle">

# Gate

**An agent-agnostic quality harness - the umpire for any AI agent's dev loop.**

[![Release](https://img.shields.io/github/v/release/brolyssjl/gate)](https://github.com/brolyssjl/gate/releases)
[![CI](https://github.com/brolyssjl/gate/actions/workflows/ci.yml/badge.svg)](https://github.com/brolyssjl/gate/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/github/license/brolyssjl/gate)](LICENSE)

</div>

## What it is

AI agents doing dev work tend to skip steps under pressure: they call a bug
fixed without reproducing it, fake or skim test evidence, and retry a failing
check forever hoping it eventually passes. Gate stops that by turning the
development flow into an enforced state machine: `PLAN → IMPLEMENT → TEST →
REVIEW → RETRO → DONE` (a bugfix run swaps IMPLEMENT for DEBUG). Any agent or
human does the thinking; Gate holds the state, checks the evidence itself
instead of trusting what it's told, and refuses to advance until that
evidence is real.

Every override - a skip, a re-trust, a streak reset - is a distinct, reasoned,
recorded act, so a run's audit trail shows what actually happened, not a log
an agent could edit into looking clean. And because Gate is the one thing
standing between an agent and "just keep retrying," it caps consecutive
failures on the same gate and stops evaluating until a human clears it.

Gate is an **umpire, not a driver.** It never invokes agents or LLMs, makes no
network calls, and has no telemetry. It holds state, verifies evidence, and
returns exit codes.

## Install

Prebuilt binaries ship with every release (`linux-x64`, `darwin-arm64`).
`install.sh` fetches the right one for your platform and puts it on your
PATH - no Node required:

```bash
curl -fsSL https://raw.githubusercontent.com/brolyssjl/gate/main/install.sh | bash
```

`GATE_VERSION=X.Y.Z ./install.sh` pins an exact version instead of installing
the latest release.

Building from source needs only a stable Rust toolchain:

```bash
git clone https://github.com/brolyssjl/gate.git
cd gate
cargo build --release --manifest-path rust/Cargo.toml
# -> rust/target/release/gate; put it on your PATH
```

## Quick start

```bash
gate init                     # scaffold .gate/, infer commands, install CLAUDE.md/AGENTS.md pointers
gate trust                    # review .gate/config.yml, then approve its commands (TOFU)
gate start "add password reset"    # optional: --profile feature|bugfix|refactor|docs
#   → enters PLAN, scaffolds .gate/runs/<id>/plan.md, prints the plan playbook
# …fill in plan.md: goal, files, acceptance criteria (each with a verify)…
gate approve                  # record sign-off (separate from writing the plan)
gate next                     # PLAN gate: schema valid? at least one checkable criterion? approved?
```

From here on, the agent loop is just two commands, repeated phase by phase:

```bash
gate playbook   # what should I do in this phase?
# …do the work…
gate next       # am I done? (runs the gate; advances on pass)
```

Everything works over the shell and `--json` output, and all state lives on
disk under `.gate/`, so it survives context compaction, crashes, and
switching agents mid-task. Keep looping `gate playbook` / do the work /
`gate next` until the run reaches DONE. When you disagree with a gate, don't
fight it: `gate check` explains exactly what evidence is missing, and fixing
that evidence is almost always faster than arguing with it.

## How it works

A run walks a fixed sequence of phases, and each phase has one deterministic
gate that checks real evidence (a plan hash, a build exit code, a signed
review) before letting you advance. Judgment - plan quality, review depth -
lives in editable playbooks the CLI hands you; only the mechanical checks
live in the gate itself.

Which phases a run walks depends on its **profile**: `feature` (the
default), `bugfix`, `refactor`, or `docs`. You pick one with `gate start
--profile <name>`; a bugfix run swaps IMPLEMENT for a DEBUG phase that
enforces reproduce-then-fix instead of just fix.

| Phase | What the gate checks |
|---|---|
| **PLAN** | The plan has a valid goal, files, and at least one checkable acceptance criterion, and a human has run `gate approve` |
| **IMPLEMENT** (or **DEBUG** for a bugfix) | You touched only files the plan declared, and your build and lint commands exit 0 |
| **TEST** | Your test command exits 0, every criterion maps to a real passing test, and diff coverage clears the threshold |
| **REVIEW** | A reviewer signed off on the exact code that shipped, with no open blocking findings |
| **RETRO** | You recorded at least one lesson learned, synced to project memory if you keep one |

See [Gates and profiles](docs/gates-and-profiles.md) for the full table and
which phases each profile (`feature`, `bugfix`, `refactor`, `docs`) runs.

## Learn more

- [Gates and profiles](docs/gates-and-profiles.md) - the full gate-by-gate
  checklist and which phases each profile runs. Read this to know exactly
  what will block you.
- [Runs and concurrency](docs/runs-and-concurrency.md) - branch-keyed runs,
  multi-stack targets, ignoring environment noise, and pruning old runs.
  Read this once you have more than one branch or stack in play.
- [Review and report](docs/review-and-report.md) - review packets, `gate
  report`, retro and the Agnosgram journal, and what happens when a plan
  drifts after approval. Read this when you reach REVIEW or RETRO.
- [Integrity](docs/integrity.md) - how Gate produces its own evidence, the
  failure-streak cap, identity fallback, and command trust (TOFU). Read this
  to understand what Gate actually guarantees.
- [Threat model](docs/threat-model.md) - what Gate defends against, what it
  doesn't, and how it treats agent-facing text as data, not instructions.
  Read this before relying on Gate for anything security-sensitive.
- [Configuration](docs/configuration.md) - the full `config.yml` reference,
  output formats, agent adapters, `gate doctor`/`gate update`, and the
  optional `gate guard` pre-commit hook. Read this to customize your setup.
- [SDD composition](docs/sdd-composition.md) - how Gate composes with
  openspec, spec-kit, or BMAD instead of competing with them. Read this if
  you already use a spec-driven-development framework.
- [ROADMAP.md](ROADMAP.md) - what's built and what's next.

## Development

```bash
cargo build --release --manifest-path rust/Cargo.toml   # -> rust/target/release/gate
cargo test --manifest-path rust/Cargo.toml               # unit tests + conformance suite
cargo clippy --all-targets --manifest-path rust/Cargo.toml -- -D warnings
```

Rust is the only implementation; there is no npm package. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the branching, PR, and release process.

## License

[MIT](LICENSE)
