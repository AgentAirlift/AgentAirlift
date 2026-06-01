# Agent Airlift — Codex plugin

Adds two slash commands to Codex:

- `/airlift-check` — refresh the `codex` provider-health signal directly from
  Marginlab, with cached fallback. If degraded, recommends airlifting to Claude Code.
- `/airlift-migrate` — run the Agent Airlift migration pipeline on the current
  session and produce a Claude Code-ready handoff bundle.

These are prompt-based slash commands: each prompt instructs Codex to run the
bundled shell script (which calls the `agent-airlift` binary) and then act on the
output. The migration pipeline itself is unchanged.

## Prerequisites

- Build the CLI: `cargo build --release` in the repo root.
- Make the binary discoverable, either:
  - put it on `PATH`, or
  - export `AGENT_AIRLIFT_BIN=/abs/path/to/target/release/agent-airlift`.
- Run Codex from the Agent Airlift repo root for the demo (the scripts use
  repo-relative paths for the cached signal and session fixtures).

## Install (dev)

Codex discovers custom prompt slash commands from `~/.codex/prompts/`. Symlink or
copy the prompts there:

```bash
mkdir -p ~/.codex/prompts
ln -sf "$(pwd)/plugins/codex/prompts/airlift-check.md"   ~/.codex/prompts/airlift-check.md
ln -sf "$(pwd)/plugins/codex/prompts/airlift-migrate.md" ~/.codex/prompts/airlift-migrate.md
```

Then use `/airlift-check` and `/airlift-migrate` inside Codex.

> Codex's packaged-plugin format is still firming up; prompt slash commands are
> the stable mechanism used here. Packaging can move to Codex's plugin layout
> later without changing the scripts.

## Configuration (env)

| Var | Default | Purpose |
|-----|---------|---------|
| `AGENT_AIRLIFT_BIN` | `agent-airlift` | Path to the CLI binary |
| `AIRLIFT_OUT` | `./airlift-out` | Output directory |
| `AIRLIFT_HEALTH_CACHE` | `examples/provider-health/degraded.marginlab.cached.codex.json` | Cached signal used as fallback / migration signal |

## Demo note

The health check queries Marginlab directly and reports whatever the live
tracker currently reports. If Marginlab cannot be reached, the script falls back
to the checked-in cached degraded signal.

## Session detection

`/airlift-migrate` picks the newest `rollout-*.jsonl` under
`~/.codex/sessions/`. If none is found, it falls back to
`examples/sessions/codex-realistic.jsonl`.
