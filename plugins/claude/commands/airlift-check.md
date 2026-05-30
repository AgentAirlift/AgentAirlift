---
description: Check whether Claude Code is degraded and recommend airlifting if so.
allowed-tools: Bash
---
Agent Airlift provider-health check:

!`"${CLAUDE_PLUGIN_ROOT}/scripts/check.sh"`

Read the `AIRLIFT_HEALTH` line above.
- If `status=degraded`: tell me Claude Code currently looks degraded (mention the
  confidence and source), and that I can run `/airlift-migrate` to airlift this
  session to Codex.
- Otherwise: report the status in one line and stop.
