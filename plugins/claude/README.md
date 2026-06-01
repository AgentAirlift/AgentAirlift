# Agent Airlift — Claude Code plugin

Adds two slash commands to Claude Code:

- `/airlift-check` — refresh the `claude-code` provider-health signal directly
  from Marginlab, with cached fallback. If degraded, recommends airlifting to Codex.
- `/airlift-migrate` — run the Agent Airlift migration pipeline on the current
  session and produce a Codex-ready handoff bundle.

The migration pipeline itself is unchanged; these commands are thin front-ends
that shell out to the `agent-airlift` binary.

## Prerequisites

- Build the CLI: `cargo build --release` in the repo root.
- Make the binary discoverable, either:
  - put it on `PATH`, or
  - export `AGENT_AIRLIFT_BIN=/abs/path/to/target/release/agent-airlift`.
- Run Claude Code from the Agent Airlift repo root for the demo (the scripts use
  repo-relative paths for the cached signal and session fixtures).

## Install (dev)

Load the plugin directory for a session:

```bash
claude --plugin-dir plugins/claude
```

Then use `/airlift-check` and `/airlift-migrate`.

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
