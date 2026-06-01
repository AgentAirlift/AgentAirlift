#!/usr/bin/env bash
# Agent Airlift — Claude Code migration.
# Locates the current live Claude Code session for this directory and airlifts it
# to the target harness (default: codex) via the Agent Airlift migration pipeline.
set -euo pipefail

BIN="${AGENT_AIRLIFT_BIN:-agent-airlift}"
TARGET="${1:-codex}"
CACHE="${AIRLIFT_HEALTH_CACHE:-examples/provider-health/degraded.marginlab.cached.json}"
OUT="${AIRLIFT_OUT:-./airlift-out}"

# Claude Code stores sessions under ~/.claude/projects/<cwd with '/' -> '-'>/
proj_dir="$HOME/.claude/projects/$(pwd | sed 's#/#-#g')"
session="$(ls -t "$proj_dir"/*.jsonl 2>/dev/null | head -n1 || true)"
if [ -z "$session" ]; then
  session="examples/sessions/claude-code-realistic.jsonl"
  echo "No live Claude Code session found for this directory; using fixture: $session" >&2
fi

"$BIN" migrate \
  --session "$session" \
  --project . \
  --out "$OUT" \
  --source claude-code \
  --targets "$TARGET" \
  --provider-health file \
  --provider-health-file "$CACHE"
