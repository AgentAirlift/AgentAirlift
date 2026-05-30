use serde_json::{json, Value};
use crate::session_import::SessionTurn;

/// Canonical, deterministic representation of a single turn.
///
/// NOTE: This captures only observable conversation data (role, text, tool
/// calls/results, timestamps, provenance). It deliberately does NOT attempt to
/// reconstruct any model hidden state or chain-of-thought — that information is
/// not present in exported sessions and is not invented here.
#[derive(Debug, Default, serde::Serialize)]
pub struct CanonicalTurn {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    /// Provenance: which source format/provider this turn came from.
    pub source: String,
    pub content_blocks: Value,
    pub tool_calls: Value,
    pub tool_results: Value,
    pub metadata: Value,
}

/// Top-level keys that are mapped into canonical fields (not copied to metadata).
const MAPPED_TOP_KEYS: &[&str] =
    &["id", "uuid", "role", "content", "timestamp", "type", "message", "payload"];

pub fn normalize_turns(
    turns: Vec<SessionTurn>,
    source: &str,
) -> (Vec<CanonicalTurn>, Vec<String>, Value) {
    let mut canonical_turns = Vec::new();
    let warnings: Vec<String> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();

    for (i, turn) in turns.into_iter().enumerate() {
        let id = turn.id.clone().unwrap_or_else(|| format!("turn-{}", i + 1));
        let role = turn.role.clone().unwrap_or_else(|| "unknown".to_string());
        let content = turn.content.clone().unwrap_or_default();
        let timestamp = turn
            .timestamp
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

        // metadata = unknown top-level keys (preserved verbatim)
        let mut metadata = json!({});
        if let Value::Object(obj) = &turn.raw {
            for (key, value) in obj {
                if !MAPPED_TOP_KEYS.contains(&key.as_str()) {
                    metadata[key] = value.clone();
                }
            }
            // record nested sub-keys we summarized rather than elevated
            collect_unmapped(obj.get("message"), "message", &mut dropped);
            collect_unmapped(obj.get("payload"), "payload", &mut dropped);
        }

        // deterministic content blocks
        let mut blocks: Vec<Value> = Vec::new();
        if !content.is_empty() {
            blocks.push(json!({"type": "text", "text": content}));
        }
        for tc in &turn.tool_calls {
            blocks.push(json!({"type": "tool_use", "tool": tc}));
        }
        for tr in &turn.tool_results {
            blocks.push(json!({"type": "tool_result", "result": tr}));
        }

        canonical_turns.push(CanonicalTurn {
            id,
            role,
            content,
            timestamp,
            source: source.to_string(),
            content_blocks: Value::Array(blocks),
            tool_calls: Value::Array(turn.tool_calls),
            tool_results: Value::Array(turn.tool_results),
            metadata,
        });
    }

    dropped.sort();
    dropped.dedup();
    let dropped_fields = json!({
        "unmapped_nested_fields": dropped,
        "note": "These source sub-fields were summarized into canonical fields, not elevated verbatim. The full original is preserved in raw/source-session.jsonl.",
    });

    (canonical_turns, warnings, dropped_fields)
}

/// Records keys inside a nested `message`/`payload` object that aren't mapped
/// to a canonical field, as `prefix.key`, so audits show what was summarized.
fn collect_unmapped(nested: Option<&Value>, prefix: &str, out: &mut Vec<String>) {
    if let Some(Value::Object(obj)) = nested {
        for key in obj.keys() {
            match key.as_str() {
                "role" | "content" | "type" | "message" => {}
                other => out.push(format!("{}.{}", prefix, other)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_canonical_session_generation() {
        let turns = vec![
            SessionTurn {
                raw: json!({
                    "id": "test-1",
                    "role": "user",
                    "content": "Test content 1",
                    "timestamp": "2024-01-01T10:00:00Z",
                    "extra_field": "extra_value"
                }),
                id: Some("test-1".to_string()),
                role: Some("user".to_string()),
                content: Some("Test content 1".to_string()),
                timestamp: Some("2024-01-01T10:00:00Z".to_string()),
                ..Default::default()
            },
            SessionTurn {
                raw: json!({
                    "id": "test-2",
                    "role": "assistant",
                    "content": "Test response 1",
                    "timestamp": "2024-01-01T10:00:05Z"
                }),
                id: Some("test-2".to_string()),
                role: Some("assistant".to_string()),
                content: Some("Test response 1".to_string()),
                timestamp: Some("2024-01-01T10:00:05Z".to_string()),
                ..Default::default()
            },
        ];

        let (canonical_turns, warnings, dropped_fields) = normalize_turns(turns, "claude-code");

        assert_eq!(canonical_turns.len(), 2);
        assert!(warnings.is_empty());
        assert!(dropped_fields.is_object());

        let first = &canonical_turns[0];
        assert_eq!(first.id, "test-1");
        assert_eq!(first.role, "user");
        assert_eq!(first.content, "Test content 1");
        assert_eq!(first.timestamp, "2024-01-01T10:00:00Z");
        assert_eq!(first.source, "claude-code"); // provenance stamped
        assert_eq!(first.metadata["extra_field"], "extra_value"); // unknown field preserved
        assert_eq!(canonical_turns[1].id, "test-2");
    }

    #[test]
    fn test_canonical_with_missing_fields() {
        let turns = vec![SessionTurn {
            raw: json!({ "content": "Test without id or role" }),
            content: Some("Test without id or role".to_string()),
            ..Default::default()
        }];

        let (canonical_turns, warnings, _) = normalize_turns(turns, "flat");
        assert_eq!(canonical_turns.len(), 1);
        assert!(warnings.is_empty());
        let c = &canonical_turns[0];
        assert_eq!(c.id, "turn-1"); // stable generated ID
        assert_eq!(c.role, "unknown"); // default role
        assert_eq!(c.content, "Test without id or role");
        assert!(!c.timestamp.is_empty());
    }

    #[test]
    fn test_canonical_preserves_tool_calls_and_blocks() {
        let turns = vec![SessionTurn {
            raw: json!({"type": "assistant", "message": {"role": "assistant", "model": "claude-sonnet-4"}}),
            id: Some("a-1".to_string()),
            role: Some("assistant".to_string()),
            content: Some("Reading the file.".to_string()),
            timestamp: Some("2026-05-29T18:00:12Z".to_string()),
            tool_calls: vec![json!({"id": "toolu_01", "name": "Read", "input": {"file_path": "src/main.rs"}})],
            tool_results: vec![],
            ..Default::default()
        }];

        let (canonical, _, dropped) = normalize_turns(turns, "claude-code");
        let c = &canonical[0];
        // tool calls preserved as first-class field
        assert_eq!(c.tool_calls.as_array().unwrap().len(), 1);
        assert_eq!(c.tool_calls[0]["name"], "Read");
        // content_blocks include a text block + tool_use block
        let blocks = c.content_blocks.as_array().unwrap();
        assert!(blocks.iter().any(|b| b["type"] == "text"));
        assert!(blocks.iter().any(|b| b["type"] == "tool_use"));
        // message.model was summarized → recorded in dropped/unmapped
        let unmapped = dropped["unmapped_nested_fields"].as_array().unwrap();
        assert!(unmapped.iter().any(|f| f == "message.model"));
    }
}
