#!/usr/bin/env bash
# Agent Airlift — Claude Code migration.
# Locates the current live Claude Code session for this directory and airlifts it
# to the target harness (default: codex) via the Agent Airlift migration pipeline.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"
BIN="${AGENT_AIRLIFT_BIN:-agent-airlift}"
TARGET="${1:-codex}"
CACHE="${AIRLIFT_HEALTH_CACHE:-$ROOT/examples/provider-health/degraded.marginlab.cached.json}"
OUT="${AIRLIFT_OUT:-./airlift-out}"
PROJECT_CWD="$(pwd -P)"

# Claude Code stores sessions under ~/.claude/projects/<cwd with '/' -> '-'>/
proj_dir="$HOME/.claude/projects/$(printf '%s' "$PROJECT_CWD" | sed 's#/#-#g')"
session="$(
  python3 - "$proj_dir" <<'PY'
import sys
from pathlib import Path

root = Path(sys.argv[1])
candidates = []
if root.exists():
    for path in root.glob("*.jsonl"):
        if path.is_file():
            candidates.append((path.stat().st_mtime, str(path)))
if candidates:
    print(max(candidates)[1])
PY
)"
if [ -z "$session" ]; then
  if [ "${AIRLIFT_ALLOW_FIXTURE_FALLBACK:-}" = "1" ]; then
    session="$ROOT/examples/sessions/claude-code-realistic.jsonl"
    echo "No live Claude Code session found for $PROJECT_CWD; using fixture: $session" >&2
  else
    echo "No live Claude Code session found for $PROJECT_CWD. Set AIRLIFT_ALLOW_FIXTURE_FALLBACK=1 to run the demo fixture explicitly." >&2
    exit 2
  fi
fi

"$BIN" migrate \
  --session "$session" \
  --project . \
  --out "$OUT" \
  --source claude-code \
  --targets "$TARGET" \
  --provider-health file \
  --provider-health-file "$CACHE"
