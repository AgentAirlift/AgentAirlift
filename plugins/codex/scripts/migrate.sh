#!/usr/bin/env bash
# Agent Airlift — Codex migration.
# Locates the current project's Codex session rollout and airlifts it to the target
# harness (default: claude-code) via the Agent Airlift migration pipeline.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd -P)"
BIN="${AGENT_AIRLIFT_BIN:-agent-airlift}"
TARGET="${1:-claude-code}"
CACHE="${AIRLIFT_HEALTH_CACHE:-$ROOT/examples/provider-health/degraded.marginlab.cached.codex.json}"
OUT="${AIRLIFT_OUT:-./airlift-out}"
PROJECT_CWD="$(pwd -P)"

# Codex stores sessions under ~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl
session="$(
  python3 - "$HOME/.codex/sessions" "$PROJECT_CWD" <<'PY'
import json
import os
import sys
from pathlib import Path

root = Path(sys.argv[1])
project_cwd = os.path.realpath(sys.argv[2])
candidates = []

if root.exists():
    for path in root.rglob("rollout-*.jsonl"):
        if not path.is_file():
            continue
        try:
            with path.open("r", encoding="utf-8") as f:
                first = f.readline()
            record = json.loads(first)
            session_cwd = record.get("payload", {}).get("cwd")
            if session_cwd and os.path.realpath(os.path.expanduser(session_cwd)) == project_cwd:
                candidates.append((path.stat().st_mtime, str(path)))
        except Exception:
            continue

if candidates:
    print(max(candidates)[1])
PY
)"
if [ -z "$session" ]; then
  if [ "${AIRLIFT_ALLOW_FIXTURE_FALLBACK:-}" = "1" ]; then
    session="$ROOT/examples/sessions/codex-realistic.jsonl"
    echo "No live Codex session found for $PROJECT_CWD; using fixture: $session" >&2
  else
    echo "No live Codex session found for $PROJECT_CWD. Set AIRLIFT_ALLOW_FIXTURE_FALLBACK=1 to run the demo fixture explicitly." >&2
    exit 2
  fi
fi

"$BIN" migrate \
  --session "$session" \
  --project . \
  --out "$OUT" \
  --source codex \
  --targets "$TARGET" \
  --provider-health file \
  --provider-health-file "$CACHE"
