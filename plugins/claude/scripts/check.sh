#!/usr/bin/env bash
# Agent Airlift — Claude Code health check.
# Refreshes the provider-health signal for claude-code via Apify, falling back
# to the checked-in degraded cache when no live signal is available.
set -euo pipefail

BIN="${AGENT_AIRLIFT_BIN:-agent-airlift}"
CACHE="${AIRLIFT_HEALTH_CACHE:-examples/provider-health/degraded.apify.cached.json}"
OUT="${AIRLIFT_OUT:-./airlift-out}"

"$BIN" health \
  --source claude-code \
  --provider-health apify \
  --apify-cache-file "$CACHE" \
  --out "$OUT"
