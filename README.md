# Agent Airlift

A local, deterministic CLI that performs a **manual transfer** of an AI coding
session from one agent to another (e.g. Claude Code → Codex / Kiro / OpenCode).
It imports a session, normalizes it to a canonical schema, and emits handoff
docs plus readable, target-shaped exports. Optional Box upload and Apify
provider-health signal ingestion are supported but never required.

> Scope note: The core CLI is deterministic and calls **no AI APIs** during
> generation. Transfers are always **explicit**. Optional Claude Code / Codex
> plugins (see [`plugins/`](plugins/)) add **on-demand** provider-health checks
> and recommend a transfer when a provider looks degraded — but they only ever
> invoke the same explicit migration workflow described here.

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

`demo` is an alias of `migrate` (the full pipeline). A lightweight `health`
subcommand runs only the provider-health step and persists
`<out>/provider-health.json` — useful for the plugins below:

```bash
cargo run -- health --source claude-code \
  --provider-health apify \
  --apify-cache-file examples/provider-health/degraded.apify.cached.json
# prints the normalized signal + a line:
# AIRLIFT_HEALTH status=degraded provider=claude-code confidence=0.74 source=cached-apify
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

## Plugins (optional)

On-demand Claude Code and Codex plugins live under [`plugins/`](plugins/) and add
two slash commands each:

- `/airlift-check` — refresh the provider-health signal; if the active provider
  looks **degraded**, recommend airlifting to the other harness.
- `/airlift-migrate` — run the (unchanged) migration pipeline on the current
  session and emit a handoff bundle for the other harness.

Pairing is cross-tool: the Claude plugin checks `claude-code` and airlifts to
`codex`; the Codex plugin checks `codex` and airlifts to `claude-code`. The
plugins are thin front-ends that shell out to `agent-airlift health` / `migrate`
— no new conversion logic. See [`plugins/SPEC.md`](plugins/SPEC.md) and each
plugin's README for install and configuration.

## Tests

```bash
cargo test
```
