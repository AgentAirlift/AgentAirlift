# Agent Airlift — Claude Code plugin

Adds two slash commands to Claude Code:

- `/airlift-check` — refresh the `claude-code` provider-health signal directly
  from Marginlab, with cached fallback. If degraded, recommends airlifting to Codex.
- `/airlift-migrate` — run the Agent Airlift migration pipeline on the current
  session and produce a Codex-ready handoff bundle.

The migration pipeline itself is unchanged; these commands are thin front-ends
that shell out to the `agent-airlift` binary.

## Prerequisites

- Make the binary discoverable, either:
  - put it on `PATH`, or
  - export `AGENT_AIRLIFT_BIN=/abs/path/to/target/release/agent-airlift`.
- Run Claude Code from the Agent Airlift repo root for the demo (the scripts use
  repo-relative paths for the cached signal and session fixtures).

## Install (dev)

Install or replace the local Claude Code plugin from the repo root:

```bash
./scripts/agent-airlift install-claude
```

The installer runs `cargo build --release`, validates the Claude plugin,
registers the repo-local marketplace, and force-refreshes
`agent-airlift@agent-airlift-local` by uninstalling the existing local copy with
`--keep-data` before installing it again.

Useful options:

```bash
./scripts/agent-airlift install-claude --skip-build
./scripts/agent-airlift install-all
```

Then restart Claude Code and use `/airlift-check` and `/airlift-migrate`.

## Configuration (env)

| Var | Default | Purpose |
|-----|---------|---------|
| `AGENT_AIRLIFT_BIN` | `agent-airlift` | Path to the CLI binary |
| `AIRLIFT_OUT` | `./airlift-out` | Output directory |
| `AIRLIFT_HEALTH_CACHE` | `examples/provider-health/degraded.marginlab.cached.json` | Cached signal used as fallback / migration signal |

## Demo note

The health check queries Marginlab directly and reports whatever the live
tracker currently reports. If Marginlab cannot be reached, the script falls back
to the checked-in cached degraded signal.

## Session detection

`/airlift-migrate` picks the newest `*.jsonl` under
`~/.claude/projects/<cwd>/`. If none is found, it falls back to
`examples/sessions/claude-code-realistic.jsonl`.
