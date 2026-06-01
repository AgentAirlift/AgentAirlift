#!/usr/bin/env bash
# Agent Airlift — Codex health check.
# Refreshes the provider-health signal for codex directly from Marginlab,
# falling back to the checked-in degraded cache when no live signal is available.
set -euo pipefail

BIN="${AGENT_AIRLIFT_BIN:-agent-airlift}"
CACHE="${AIRLIFT_HEALTH_CACHE:-examples/provider-health/degraded.marginlab.cached.codex.json}"
OUT="${AIRLIFT_OUT:-./airlift-out}"

"$BIN" health \
  --source codex \
  --provider-health marginlab \
  --provider-health-cache-file "$CACHE" \
  --out "$OUT"
