#!/usr/bin/env bash
# Agent Airlift — Codex health check.
# Refreshes the provider-health signal for codex via Apify, falling back to the
# checked-in degraded cache when no live signal is available.
set -euo pipefail

BIN="${AGENT_AIRLIFT_BIN:-agent-airlift}"
CACHE="${AIRLIFT_HEALTH_CACHE:-examples/provider-health/degraded.apify.cached.codex.json}"
OUT="${AIRLIFT_OUT:-./airlift-out}"

"$BIN" health \
  --source codex \
  --provider-health apify \
  --apify-cache-file "$CACHE" \
  --out "$OUT"
