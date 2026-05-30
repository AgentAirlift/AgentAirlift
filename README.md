# Agent Airlift

A local, deterministic CLI that performs a **manual transfer** of an AI coding
session from one agent to another (e.g. Claude Code → Codex / Kiro / OpenCode).
It imports a session, normalizes it to a canonical schema, and emits handoff
docs plus readable, target-shaped exports. Optional Box upload and Apify
provider-health signal ingestion are supported but never required.

> Scope note: Agent Airlift does **not** auto-detect provider degradation and has
> **no plugin system**. Transfers are explicit and deterministic. No AI APIs are
> called during generation.

## Quick start

```bash
cargo run -- demo \
  --session examples/sessions/claude-code-realistic.jsonl \
  --project examples/projects/tiny-rust-cli \
  --out ./airlift-out \
  --source claude-code \
  --targets codex,kiro,opencode \
  --provider-health none
```

Output is written under `airlift-out/`:

```
raw/         source session copy, repo snapshot, provider-health, apify-response
normalized/  canonical-session.json (deterministic schema)
replay/      agent-airlift.session.jsonl
exports/     HANDOFF.md, AGENTS.md, codex/kiro/opencode exports, .kiro specs
audit/       conversion-report.md, warnings.json, dropped-fields.json,
             import-diagnostics.json, upload-manifest.json (if Box used)
```

## Supported import formats

The importer tolerates JSONL variations and never panics on malformed lines —
bad rows become structured warnings. Detected format and a confidence score are
recorded in `audit/import-diagnostics.json`.

| Format        | Shape                                                                 |
|---------------|-----------------------------------------------------------------------|
| `claude-code` | `{type, message:{role, content: string \| blocks}}`; tool_use / tool_result blocks captured; `progress`/`system`/`file-history-snapshot` skipped |
| `codex`       | `session_meta` + `event_msg` (user/agent) + `response_item` (deduped) |
| `flat`        | `{id, role, content, timestamp, ...}` (unknown fields preserved)      |

Realistic example fixtures live in `examples/sessions/`:
`claude-code-realistic.jsonl`, `codex-realistic.jsonl`, `edge-cases.jsonl`.

## Canonical schema

Each canonical turn carries stable `id`, `role`, `content`, `timestamp`,
`source` (provenance), `content_blocks`, `tool_calls`, `tool_results`, and
`metadata` (preserved unknown fields). The schema is deterministic. It captures
only observable conversation data — it does **not** reconstruct model hidden
state or chain-of-thought.

## Resume compatibility

Target exports under `exports/` are readable, deterministic representations of
the session, **not** native session files. Native `resume` compatibility is not
guaranteed; use the exports plus `HANDOFF.md` / `AGENTS.md` to seed a fresh
session in the target tool.

## Provider health (optional)

```bash
# file mode (no network, no token)
--provider-health file --provider-health-file examples/provider-health/degraded.apify.cached.json

# apify mode: scrapes the live tracker; falls back to cache if token missing or run fails
# "Collecting baseline data" on the tracker page is treated as nominal (healthy)
--provider-health apify \
  --apify-actor-id apify~website-content-crawler \
  --apify-input-url https://marginlab.ai/trackers/claude-code/ \
  --apify-cache-file examples/provider-health/degraded.apify.cached.json
```

Generated docs distinguish the signal source from the evaluated provider, e.g.
*"Provider health signal from `cached-apify`: `claude-code` is degraded."*

## Box upload (optional)

```bash
--box-dry-run                      # print plan + manifest, no API calls, no token
--box-upload --box-parent-folder-id <id>   # requires BOX_DEVELOPER_TOKEN
```

The upload manifest records Box folder/file IDs and URLs. Tokens are never
written to any output artifact.

## Tests

```bash
cargo test
```
