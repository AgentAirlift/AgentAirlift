#!/usr/bin/env bash
# Agent Airlift — Codex migration.
# Locates the most recent Codex session rollout and airlifts it to the target
# harness (default: claude-code) via the Agent Airlift migration pipeline.
set -euo pipefail

BIN="${AGENT_AIRLIFT_BIN:-agent-airlift}"
TARGET="${1:-claude-code}"
CACHE="${AIRLIFT_HEALTH_CACHE:-examples/provider-health/degraded.marginlab.cached.codex.json}"
OUT="${AIRLIFT_OUT:-./airlift-out}"

# Codex stores sessions under ~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl
session="$(find "$HOME/.codex/sessions" -name 'rollout-*.jsonl' -type f 2>/dev/null \
  | xargs ls -t 2>/dev/null | head -n1 || true)"
if [ -z "$session" ]; then
  session="examples/sessions/codex-realistic.jsonl"
  echo "No live Codex session found; using fixture: $session" >&2
fi

"$BIN" migrate \
  --session "$session" \
  --project . \
  --out "$OUT" \
  --source codex \
  --targets "$TARGET" \
  --provider-health file \
  --provider-health-file "$CACHE"
