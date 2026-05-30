Run the Agent Airlift health check by executing this command in the shell:

    bash plugins/codex/scripts/check.sh

(If the plugin is installed outside the Agent Airlift repo, run `check.sh` from
the plugin directory, and set `AGENT_AIRLIFT_BIN` / `AIRLIFT_HEALTH_CACHE` if the
binary or cached signal are not on the default paths.)

Then read the `AIRLIFT_HEALTH` line in the output:
- If `status=degraded`, tell me Codex currently looks degraded (mention the
  confidence and source) and that I can run `/airlift-migrate` to airlift this
  session to Claude Code.
- Otherwise, report the status in one line and stop.
