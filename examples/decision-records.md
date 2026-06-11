# Agent Airlift Decision Records

Paste this into `CLAUDE.md` or `AGENTS.md` for projects that may be migrated
with Agent Airlift:

```markdown
## Agent Airlift Decision Records

When you make a settled implementation or product decision, record it in this
greppable format so Agent Airlift can preserve the rationale during migration:

DECISION(<stable-id>): <what is decided>
RATIONALE(<stable-id>): <why this is decided>
  <optional continuation line with supporting context>
STATUS(<stable-id>): settled - do not revisit unless requirements change.

Use short, stable IDs such as `auth-skip`, `api-v2`, or `billing-retry`.
Do not put secrets in decision records.
```

Agent Airlift extracts `DECISION`, `RATIONALE`, and `STATUS` records verbatim
into `HANDOFF.md`, `AGENTS.md`, and target-specific context exports. Malformed
records are surfaced as warnings in the preserved decision section instead of
being silently ignored.
