---
name: airlift-migrate
description: Airlift the current Codex session to Claude Code using the Agent Airlift migration pipeline. Use when the user says /airlift-migrate or asks to hand off this session to Claude Code.
---

# Airlift Migrate

Airlift this Codex session to Claude Code by running this command in the shell from the Agent Airlift repo:

    bash plugins/codex/scripts/migrate.sh claude-code

Then summarize the result:

- Point the user to the handoff bundle (`./airlift-out/exports/HANDOFF.md` and
  `./airlift-out/exports/AGENTS.md`).
- Give a short resume prompt they can paste into Claude Code to continue this
  work.
