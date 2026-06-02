---
name: airlift-check
description: Check Codex provider health directly from Marginlab via Agent Airlift and recommend airlifting to Claude Code if degraded. Use when the user says /airlift-check, asks whether Codex looks degraded, or wants a provider-health check.
---

# Airlift Check

Run the Agent Airlift health check by executing this command in the shell from the Agent Airlift repo:

    bash plugins/codex/scripts/check.sh

(If running outside the Agent Airlift repo, run `check.sh` from the plugin
directory, and set `AGENT_AIRLIFT_BIN` / `AIRLIFT_HEALTH_CACHE` if the binary or
cached signal are not on the default paths.)

Then read the `AIRLIFT_HEALTH` line in the output:

- If `status=degraded`, tell the user Codex currently looks degraded (mention the
  confidence and source) and that they can run `/airlift-migrate` to
  airlift this session to Claude Code.
- Otherwise, report the status in one line and stop.
