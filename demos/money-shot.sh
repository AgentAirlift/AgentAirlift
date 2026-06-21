#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-./airlift-demo-out}"

cd "$ROOT"

printf 'Agent Airlift money-shot demo\n'
printf 'Output: %s\n\n' "$OUT"

rm -rf "$OUT"

printf '1. Provider health says Claude Code is degraded.\n\n'
cargo run --quiet -- health \
  --source claude-code \
  --out "$OUT/health" \
  --provider-health file \
  --provider-health-file examples/provider-health/degraded.marginlab.cached.json |
  sed -n '/AIRLIFT_HEALTH/p'

printf '\n2. Airlift the session into Codex, Kiro, and OpenCode.\n\n'
cargo run --quiet -- demo \
  --session examples/sessions/claude-code-realistic.jsonl \
  --project examples/projects/tiny-rust-cli \
  --out "$OUT" \
  --source claude-code \
  --targets codex,kiro,opencode \
  --provider-health file \
  --provider-health-file examples/provider-health/degraded.marginlab.cached.json

printf '\n3. Handoff proof: the next agent gets objective, state, risks, and resume prompt.\n\n'
sed -n '1,90p' "$OUT/exports/HANDOFF.md"

printf '\n4. CI gate: migration accounting and target exports are valid.\n\n'
cat "$OUT/audit/ci-gate.json"
printf '\n'
