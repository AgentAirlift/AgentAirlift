use anyhow::Context;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// A single imported turn with structured extras preserved where available.
/// `raw` always retains the original JSON line so no information is lost.
#[derive(Debug, Default)]
pub struct SessionTurn {
    pub raw: Value,
    pub id: Option<String>,
    pub role: Option<String>,
    pub content: Option<String>,
    pub timestamp: Option<String>,
    pub record_type: String,
    pub source_line: usize,
    pub raw_sha256: String,
    pub content_blocks: Vec<Value>,
    pub tool_calls: Vec<Value>,
    pub tool_results: Vec<Value>,
}

/// Diagnostics describing how the import went, surfaced in audit artifacts.
#[derive(Debug, Default, serde::Serialize)]
pub struct ImportDiagnostics {
    pub source_path: String,
    pub lines_read: usize,
    pub turns_imported: usize,
    pub lines_skipped: usize,
    pub mapped_records: usize,
    pub intentionally_skipped_records: usize,
    pub malformed_records: usize,
    pub preserved_unknown_records: usize,
    pub accounting_balanced: bool,
    pub detected_format: String,
    pub format_confidence: f64,
    pub warnings: Vec<String>,
}

/// Recognized session formats. `Unknown` rows are tolerated, not fatal.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FormatKind {
    ClaudeCode,
    Codex,
    Flat,
    Unknown,
}

/// Codex metadata rows that carry no turn content in the observed format.
const CODEX_SKIP_TYPES: &[&str] = &["session_meta", "turn_context"];
const CLAUDE_PRESERVE_TYPES: &[&str] = &["progress", "system", "file-history-snapshot", "summary"];

pub fn import_session(path: &Path) -> anyhow::Result<(Vec<SessionTurn>, ImportDiagnostics)> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open session file: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut turns = Vec::new();
    let mut diag = ImportDiagnostics {
        source_path: path.to_string_lossy().to_string(),
        ..Default::default()
    };
    // per-format tallies for confidence scoring
    let (mut claude, mut codex, mut flat, mut unknown) = (0usize, 0usize, 0usize, 0usize);
    let mut previous_was_compact_boundary = false;

    for (i, line) in reader.lines().enumerate() {
        let line_num = i + 1;
        let line = line.with_context(|| format!("Failed to read line {}", line_num))?;
        if line.trim().is_empty() {
            continue;
        }
        diag.lines_read += 1;

        let raw: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                diag.malformed_records += 1;
                diag.warnings
                    .push(format!("Line {}: Invalid JSON - {}", line_num, e));
                continue;
            }
        };

        match classify_and_extract(raw, line_num, raw_hash(&line)) {
            LineOutcome::Turn(mut turn, kind, preserved_unknown, warning) => {
                tally(kind, &mut claude, &mut codex, &mut flat, &mut unknown);
                let is_compact_boundary = is_compact_boundary_turn(&turn);
                if previous_was_compact_boundary && is_compact_continuation_summary(&turn) {
                    turn.record_type = "compact_summary".to_string();
                    turn.role = Some("summary".to_string());
                }
                previous_was_compact_boundary = false;
                if is_duplicate_response_item(&turns, &turn) {
                    diag.lines_skipped += 1;
                    diag.intentionally_skipped_records += 1;
                    if let Some(w) = warning {
                        diag.warnings.push(w);
                    }
                    continue;
                }
                turns.push(turn);
                diag.turns_imported += 1;
                diag.mapped_records += 1;
                if preserved_unknown {
                    diag.preserved_unknown_records += 1;
                }
                if let Some(w) = warning {
                    diag.warnings.push(w);
                }
                if is_compact_boundary {
                    previous_was_compact_boundary = true;
                }
            }
            LineOutcome::Skip(kind, warning) => {
                tally(kind, &mut claude, &mut codex, &mut flat, &mut unknown);
                previous_was_compact_boundary = false;
                diag.lines_skipped += 1;
                diag.intentionally_skipped_records += 1;
                if let Some(w) = warning {
                    diag.warnings.push(w);
                }
            }
        }
    }

    // Detect dominant format + confidence
    let parsed = (claude + codex + flat + unknown).max(1);
    let (fmt, top) = [
        ("claude-code", claude),
        ("codex", codex),
        ("flat", flat),
        ("unknown", unknown),
    ]
    .into_iter()
    .max_by_key(|(_, n)| *n)
    .unwrap();
    diag.detected_format = fmt.to_string();
    diag.format_confidence = ((top as f64 / parsed as f64) * 100.0).round() / 100.0;

    diag.accounting_balanced = diag.lines_read
        == diag.mapped_records + diag.intentionally_skipped_records + diag.malformed_records;

    if turns.is_empty() {
        return Err(anyhow::anyhow!(
            "Session file is empty or contains no valid JSONL rows"
        ));
    }

    Ok((turns, diag))
}

enum LineOutcome {
    Turn(SessionTurn, FormatKind, bool, Option<String>),
    Skip(FormatKind, Option<String>),
}

fn tally(
    kind: FormatKind,
    claude: &mut usize,
    codex: &mut usize,
    flat: &mut usize,
    unknown: &mut usize,
) {
    match kind {
        FormatKind::ClaudeCode => *claude += 1,
        FormatKind::Codex => *codex += 1,
        FormatKind::Flat => *flat += 1,
        FormatKind::Unknown => *unknown += 1,
    }
}

/// Routes a parsed line to the right extractor based on shape.
fn classify_and_extract(raw: Value, line_num: usize, raw_sha256: String) -> LineOutcome {
    let entry_type = raw.get("type").and_then(|v| v.as_str()).map(str::to_string);
    if let Some(ty) = entry_type {
        match ty.as_str() {
            "user" | "assistant" => return extract_claude_message(raw, &ty, line_num, raw_sha256),
            "event_msg" => return extract_codex_event(raw, line_num, raw_sha256),
            "response_item" => return extract_codex_response_item(raw, line_num, raw_sha256),
            t if CODEX_SKIP_TYPES.contains(&t) => {
                return LineOutcome::Skip(FormatKind::Codex, None);
            }
            t if CLAUDE_PRESERVE_TYPES.contains(&t) => {
                return preserve_record(
                    raw,
                    line_num,
                    raw_sha256,
                    t,
                    FormatKind::ClaudeCode,
                    false,
                    None,
                );
            }
            other => {
                return preserve_record(
                    raw,
                    line_num,
                    raw_sha256,
                    other,
                    FormatKind::Unknown,
                    true,
                    Some(format!(
                        "Line {}: preserved unrecognized entry type '{}'",
                        line_num, other
                    )),
                );
            }
        }
    }
    // No `type` field → flat {id, role, content} shape
    if raw.get("role").is_some() || raw.get("content").is_some() {
        return extract_flat(raw, line_num, raw_sha256);
    }
    preserve_record(
        raw,
        line_num,
        raw_sha256,
        "unknown",
        FormatKind::Unknown,
        true,
        Some(format!(
            "Line {}: preserved row with no recognizable role/content/type",
            line_num
        )),
    )
}

/// Flat `{id, role, content, timestamp, ...}` rows (our demo + simple exports).
fn extract_flat(raw: Value, source_line: usize, raw_sha256: String) -> LineOutcome {
    let content_value = raw.get("content");
    let turn = SessionTurn {
        id: raw.get("id").and_then(|v| v.as_str()).map(str::to_string),
        role: raw.get("role").and_then(|v| v.as_str()).map(str::to_string),
        content: content_value.and_then(value_to_text),
        timestamp: raw
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        record_type: "flat".to_string(),
        source_line,
        raw_sha256,
        content_blocks: content_value
            .map(content_value_to_blocks)
            .unwrap_or_default(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        raw,
    };
    LineOutcome::Turn(turn, FormatKind::Flat, false, None)
}

/// Claude Code entries: `message.content` is a string or an array of typed blocks.
fn extract_claude_message(
    raw: Value,
    ty: &str,
    source_line: usize,
    raw_sha256: String,
) -> LineOutcome {
    let message = raw.get("message").cloned().unwrap_or(Value::Null);
    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or(ty)
        .to_string();

    let mut texts: Vec<String> = Vec::new();
    let mut tool_calls = Vec::new();
    let mut tool_results = Vec::new();
    let mut content_blocks = Vec::new();

    match message.get("content") {
        Some(Value::String(s)) => {
            texts.push(s.clone());
            content_blocks.push(json!({"type": "text", "text": s}));
        }
        Some(Value::Array(blocks)) => {
            for block in blocks {
                content_blocks.push(block.clone());
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            texts.push(t.to_string());
                        }
                    }
                    Some("tool_use") => {
                        tool_calls.push(block.clone());
                    }
                    Some("tool_result") => {
                        tool_results.push(block.clone());
                    }
                    _ => {}
                }
            }
        }
        Some(other) => {
            if let Some(text) = value_to_text(other) {
                texts.push(text);
            }
            content_blocks.extend(content_value_to_blocks(other));
        }
        None => {}
    }

    let content = synthesize_content(&texts, &tool_calls, &tool_results);
    let turn = SessionTurn {
        id: raw
            .get("uuid")
            .or_else(|| raw.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        role: Some(role),
        content: Some(content),
        timestamp: raw
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        record_type: ty.to_string(),
        source_line,
        raw_sha256,
        content_blocks,
        tool_calls,
        tool_results,
        raw,
    };
    LineOutcome::Turn(turn, FormatKind::ClaudeCode, false, None)
}

/// Codex `event_msg` entries carry user/agent messages under `payload`.
fn extract_codex_event(raw: Value, line_num: usize, raw_sha256: String) -> LineOutcome {
    let payload = raw.get("payload").cloned().unwrap_or(Value::Null);
    let ptype = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let role = match ptype {
        "user_message" => "user",
        "agent_message" => "assistant",
        other => {
            return preserve_record(
                raw,
                line_num,
                raw_sha256,
                "event_msg",
                FormatKind::Codex,
                true,
                Some(format!(
                    "Line {}: preserved codex event_msg payload type '{}'",
                    line_num, other
                )),
            );
        }
    };
    let content_value = payload.get("message").or_else(|| payload.get("content"));
    let content = content_value.and_then(value_to_text).unwrap_or_default();
    let turn = SessionTurn {
        id: payload
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        role: Some(role.to_string()),
        content: Some(content),
        timestamp: raw
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        record_type: "event_msg".to_string(),
        source_line: line_num,
        raw_sha256,
        content_blocks: content_value
            .map(content_value_to_blocks)
            .unwrap_or_default(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        raw,
    };
    LineOutcome::Turn(turn, FormatKind::Codex, false, None)
}

fn extract_codex_response_item(raw: Value, line_num: usize, raw_sha256: String) -> LineOutcome {
    let payload = raw.get("payload").cloned().unwrap_or(Value::Null);
    let content_value = payload.get("content").or_else(|| payload.get("message"));
    let content = content_value.and_then(value_to_text).unwrap_or_default();
    let role = payload
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("assistant")
        .to_string();
    let turn = SessionTurn {
        id: payload
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        role: Some(role),
        content: Some(content),
        timestamp: raw
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        record_type: "response_item".to_string(),
        source_line: line_num,
        raw_sha256,
        content_blocks: content_value
            .map(content_value_to_blocks)
            .unwrap_or_default(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        raw,
    };
    LineOutcome::Turn(turn, FormatKind::Codex, false, None)
}

fn preserve_record(
    raw: Value,
    line_num: usize,
    raw_sha256: String,
    record_type: &str,
    kind: FormatKind,
    preserved_unknown: bool,
    warning: Option<String>,
) -> LineOutcome {
    let content = extract_record_text(&raw).unwrap_or_else(|| raw.to_string());
    let role = match record_type {
        "summary" => "summary",
        "system" => "system",
        _ => "metadata",
    };
    if let Some(w) = warning {
        LineOutcome::Turn(
            preserved_turn(raw, line_num, raw_sha256, record_type, role, content),
            kind,
            preserved_unknown,
            Some(w),
        )
    } else {
        LineOutcome::Turn(
            preserved_turn(raw, line_num, raw_sha256, record_type, role, content),
            kind,
            preserved_unknown,
            None,
        )
    }
}

fn preserved_turn(
    raw: Value,
    line_num: usize,
    raw_sha256: String,
    record_type: &str,
    role: &str,
    content: String,
) -> SessionTurn {
    SessionTurn {
        id: raw
            .get("uuid")
            .or_else(|| raw.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        role: Some(role.to_string()),
        content: Some(content),
        timestamp: raw
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        record_type: record_type.to_string(),
        source_line: line_num,
        raw_sha256,
        content_blocks: vec![
            json!({"type": "raw_record", "record_type": record_type, "value": raw.clone()}),
        ],
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        raw,
    }
}

/// Build readable content even when a turn is only tool calls/results.
fn synthesize_content(texts: &[String], tool_calls: &[Value], tool_results: &[Value]) -> String {
    if !texts.is_empty() {
        return texts.join("\n");
    }
    if !tool_results.is_empty() {
        return tool_results
            .iter()
            .map(|r| {
                let body = r
                    .get("content")
                    .and_then(value_to_text)
                    .unwrap_or_else(|| r.get("text").and_then(value_to_text).unwrap_or_default());
                let err = r.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                let label = if err {
                    "[tool result: error]"
                } else {
                    "[tool result]"
                };
                format!("{} {}", label, body)
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    if !tool_calls.is_empty() {
        let names: Vec<String> = tool_calls
            .iter()
            .filter_map(|c| c.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        return format!("[tool call: {}]", names.join(", "));
    }
    String::new()
}

fn is_duplicate_response_item(existing: &[SessionTurn], candidate: &SessionTurn) -> bool {
    if candidate.record_type != "response_item" {
        return false;
    }
    existing.iter().rev().any(|turn| {
        turn.role == candidate.role
            && turn.timestamp == candidate.timestamp
            && turn.content == candidate.content
            && turn.record_type == "event_msg"
    })
}

fn is_compact_boundary_turn(turn: &SessionTurn) -> bool {
    turn.record_type == "system"
        && turn.raw.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary")
}

fn is_compact_continuation_summary(turn: &SessionTurn) -> bool {
    if turn.record_type != "user" || turn.role.as_deref() != Some("user") {
        return false;
    }
    let content = turn.content.as_deref().unwrap_or("").to_ascii_lowercase();
    content.contains("this session is being continued")
        && content.contains("previous conversation")
        && (content.contains("summarized below") || content.contains("summary"))
}

fn raw_hash(line: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(line.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn extract_record_text(raw: &Value) -> Option<String> {
    for key in ["summary", "content", "text", "message"] {
        if let Some(text) = raw.get(key).and_then(value_to_text) {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    raw.get("message")
        .and_then(|m| m.get("content"))
        .and_then(value_to_text)
        .or_else(|| {
            raw.get("payload").and_then(|p| {
                p.get("message")
                    .or_else(|| p.get("content"))
                    .and_then(value_to_text)
            })
        })
        .or_else(|| {
            raw.get("data").and_then(|d| {
                d.get("summary")
                    .or_else(|| d.get("content"))
                    .and_then(value_to_text)
            })
        })
}

fn value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(value_to_text).collect();
            if parts.is_empty() {
                Some(value.to_string())
            } else {
                Some(parts.join("\n"))
            }
        }
        Value::Object(obj) => obj
            .get("text")
            .or_else(|| obj.get("message"))
            .or_else(|| obj.get("content"))
            .and_then(value_to_text)
            .or_else(|| Some(Value::Object(obj.clone()).to_string())),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn content_value_to_blocks(value: &Value) -> Vec<Value> {
    match value {
        Value::String(s) => vec![json!({"type": "text", "text": s})],
        Value::Array(items) => items.clone(),
        other => vec![json!({"type": "raw_content", "value": other})],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_import_session_parses_expected_turns() {
        let path = Path::new("examples/sessions/sample-claude-like.jsonl");
        let (turns, diag) = import_session(path).unwrap();
        assert_eq!(
            turns.len(),
            6,
            "Should parse exactly 6 turns from sample session"
        );
        assert!(
            diag.warnings.is_empty(),
            "Should have no warnings for valid JSONL"
        );
        assert_eq!(diag.detected_format, "flat");
        let first_turn = &turns[0];
        assert_eq!(first_turn.id.as_deref(), Some("turn-1"));
        assert_eq!(first_turn.role.as_deref(), Some("user"));
        assert!(first_turn
            .content
            .as_deref()
            .unwrap()
            .contains("Create a simple Rust CLI"));
    }

    #[test]
    fn test_invalid_jsonl_produces_warnings_not_panics() {
        let temp_dir = TempDir::new().unwrap();
        let invalid_jsonl_path = temp_dir.path().join("invalid.jsonl");
        fs::write(&invalid_jsonl_path, "{\"id\": \"valid-1\", \"role\": \"user\", \"content\": \"test\"}\ninvalid json here\n{\"id\": \"valid-2\", \"role\": \"assistant\", \"content\": \"response\"}").unwrap();

        let (turns, diag) = import_session(&invalid_jsonl_path).unwrap();
        assert_eq!(turns.len(), 2, "Should parse 2 valid turns");
        assert_eq!(
            diag.warnings.len(),
            1,
            "Should have 1 warning for invalid line"
        );
        assert!(diag.warnings[0].contains("Invalid JSON"));
        assert_eq!(turns[0].id.as_deref(), Some("valid-1"));
        assert_eq!(turns[1].id.as_deref(), Some("valid-2"));
        assert!(diag.accounting_balanced);
        assert_eq!(
            diag.lines_read,
            diag.mapped_records + diag.intentionally_skipped_records + diag.malformed_records
        );
    }

    #[test]
    fn test_invalid_only_session_fails() {
        let temp_dir = TempDir::new().unwrap();
        let invalid_jsonl_path = temp_dir.path().join("invalid-only.jsonl");
        fs::write(&invalid_jsonl_path, "not json\nalso not json").unwrap();

        let err = import_session(&invalid_jsonl_path).unwrap_err();
        assert!(err.to_string().contains("no valid JSONL rows"));
    }

    #[test]
    fn test_claude_code_nested_format_with_tool_calls() {
        let path = Path::new("examples/sessions/claude-code-realistic.jsonl");
        let (turns, diag) = import_session(path).unwrap();
        assert_eq!(diag.detected_format, "claude-code");
        assert!(
            diag.format_confidence > 0.9,
            "confidence should be high, got {}",
            diag.format_confidence
        );
        assert!(diag.accounting_balanced);
        assert_eq!(
            diag.lines_read,
            diag.mapped_records + diag.intentionally_skipped_records + diag.malformed_records
        );
        assert!(turns.len() >= 9, "got {} turns", turns.len());

        // The first assistant turn used a Read tool
        let assistant_with_tool = turns.iter().find(|t| !t.tool_calls.is_empty()).unwrap();
        assert_eq!(assistant_with_tool.tool_calls[0]["name"], "Read");

        // There is an error tool_result somewhere
        let has_error_result = turns.iter().any(|t| {
            t.tool_results
                .iter()
                .any(|r| r["is_error"].as_bool() == Some(true))
        });
        assert!(has_error_result, "should capture an is_error tool_result");
    }

    #[test]
    fn test_codex_event_msg_format() {
        let path = Path::new("examples/sessions/codex-realistic.jsonl");
        let (turns, diag) = import_session(path).unwrap();
        assert_eq!(diag.detected_format, "codex");
        // session_meta is skipped and duplicate response_item rows are de-duplicated.
        assert!(diag.lines_skipped >= 1);
        assert_eq!(turns.len(), 4, "got {} turns", turns.len());
        assert_eq!(turns[0].role.as_deref(), Some("user"));
        assert_eq!(turns[1].role.as_deref(), Some("assistant"));
        assert!(!turns.iter().any(|t| t.record_type == "response_item"));
    }

    #[test]
    fn test_unsupported_codex_event_payload_is_preserved() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().join("codex-tool-event.jsonl");
        fs::write(
            &session_path,
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"tool_output","message":"tool output must survive","details":{"exit_code":1}}}"#,
        ).unwrap();

        let (turns, diag) = import_session(&session_path).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].record_type, "event_msg");
        assert_eq!(turns[0].role.as_deref(), Some("metadata"));
        assert_eq!(turns[0].raw["payload"]["details"]["exit_code"], 1);
        assert!(turns[0]
            .content
            .as_deref()
            .unwrap()
            .contains("tool output must survive"));
        assert_eq!(diag.lines_skipped, 0);
        assert_eq!(diag.preserved_unknown_records, 1);
        assert!(diag.warnings.iter().any(|w| w.contains("tool_output")));
    }

    #[test]
    fn test_edge_cases_tolerated_with_structured_warnings() {
        let path = Path::new("examples/sessions/edge-cases.jsonl");
        let (turns, diag) = import_session(path).unwrap();
        assert!(turns.len() >= 4, "got {} turns", turns.len());
        // unknown fields preserved in raw
        assert_eq!(turns[0].raw["unmapped_top"], "keep-me");
        assert_eq!(turns[0].raw["custom_meta"]["client"], "cli");
        // 2 invalid-json lines + 1 unknown-type telemetry warning = 3 warnings
        assert!(
            diag.warnings
                .iter()
                .filter(|w| w.contains("Invalid JSON"))
                .count()
                == 2
        );
        assert!(diag.warnings.iter().any(|w| w.contains("telemetry")));
        assert!(turns
            .iter()
            .any(|t| t.record_type == "telemetry" && t.raw["payload"]["event"] == "heartbeat"));
        assert!(diag.accounting_balanced);
    }

    #[test]
    fn test_compaction_summary_and_unknown_records_survive() {
        let path = Path::new("examples/sessions/compact-boundary.jsonl");
        let (turns, diag) = import_session(path).unwrap();

        assert!(diag.accounting_balanced);
        assert!(turns.iter().any(|t| {
            t.record_type == "summary"
                && t.role.as_deref() == Some("summary")
                && t.content.as_deref().unwrap_or("").contains("older context")
        }));
        assert!(turns.iter().any(|t| {
            t.record_type == "future_event"
                && t.role.as_deref() == Some("metadata")
                && t.raw["payload"]["note"] == "preserve unknown payload"
        }));
        assert!(diag.preserved_unknown_records >= 1);
    }

    #[test]
    fn test_real_claude_compact_boundary_marks_next_continuation_as_summary() {
        let path = Path::new("examples/sessions/claude-compact-boundary-realistic.jsonl");
        let (turns, diag) = import_session(path).unwrap();

        assert!(diag.accounting_balanced);
        assert!(turns.iter().any(|t| {
            t.record_type == "system"
                && t.role.as_deref() == Some("system")
                && t.raw["subtype"] == "compact_boundary"
                && t.content
                    .as_deref()
                    .unwrap_or("")
                    .contains("Compact boundary")
        }));

        let summary = turns
            .iter()
            .find(|t| t.id.as_deref() == Some("u-real-compact-summary"))
            .expect("compact continuation summary turn");
        assert_eq!(summary.record_type, "compact_summary");
        assert_eq!(summary.role.as_deref(), Some("summary"));
        assert!(summary
            .content
            .as_deref()
            .unwrap_or("")
            .contains("DECISION(api-v2): Keep API v2"));
        assert_eq!(summary.raw["message"]["role"], "user");
    }

    #[test]
    fn test_structured_flat_content_survives_as_blocks() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().join("structured.jsonl");
        fs::write(
            &session_path,
            r#"{"id":"s1","role":"assistant","content":[{"type":"text","text":"DECISION: keep v2"},{"type":"image","url":"file.png"}],"timestamp":"2026-01-01T00:00:00Z"}"#,
        ).unwrap();

        let (turns, _) = import_session(&session_path).unwrap();
        assert_eq!(turns.len(), 1);
        assert!(turns[0]
            .content
            .as_deref()
            .unwrap()
            .contains("DECISION: keep v2"));
        assert_eq!(turns[0].content_blocks.len(), 2);
        assert_eq!(turns[0].content_blocks[1]["type"], "image");
    }

    #[test]
    fn test_claude_object_content_survives_as_raw_content_block() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().join("object-content.jsonl");
        fs::write(
            &session_path,
            r#"{"type":"assistant","uuid":"a1","timestamp":"2026-01-01T00:00:00Z","message":{"role":"assistant","content":{"type":"custom","text":"custom object text","extra":{"keep":true}}}}"#,
        ).unwrap();

        let (turns, _) = import_session(&session_path).unwrap();
        assert_eq!(turns.len(), 1);
        assert!(turns[0]
            .content
            .as_deref()
            .unwrap()
            .contains("custom object text"));
        assert_eq!(turns[0].content_blocks[0]["type"], "raw_content");
        assert_eq!(turns[0].content_blocks[0]["value"]["extra"]["keep"], true);
    }

    #[test]
    fn test_claude_tool_result_array_content_is_synthesized() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().join("tool-result-array.jsonl");
        fs::write(
            &session_path,
            r#"{"type":"user","uuid":"u1","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","is_error":true,"content":[{"type":"text","text":"line one"},{"type":"text","text":"line two"}]}]}}"#,
        ).unwrap();

        let (turns, _) = import_session(&session_path).unwrap();
        assert_eq!(turns.len(), 1);
        assert!(turns[0].content.as_deref().unwrap().contains("line one"));
        assert!(turns[0].content.as_deref().unwrap().contains("line two"));
    }

    #[test]
    fn test_unknown_record_raw_remains_source_payload() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().join("unknown.jsonl");
        fs::write(
            &session_path,
            r#"{"type":"future_event","payload":{"note":"preserve me"}}"#,
        )
        .unwrap();

        let (turns, diag) = import_session(&session_path).unwrap();
        assert_eq!(turns.len(), 1);
        assert!(diag.warnings.iter().any(|w| w.contains("future_event")));
        assert!(turns[0].raw.get("airlift_warning").is_none());
        assert_eq!(turns[0].raw["payload"]["note"], "preserve me");
    }

    #[test]
    fn test_adversarial_jsonl_corpus_is_fully_accounted() {
        let temp_dir = TempDir::new().unwrap();
        let session_path = temp_dir.path().join("adversarial.jsonl");
        fs::write(
            &session_path,
            concat!(
                "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"},{\"type\":\"unknown_block\",\"payload\":{\"keep\":true}}]}}\n",
                "{\"type\":\"future_event\",\"payload\":{\"nested\":{\"keep\":[1,true,{\"x\":\"y\"}]}}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"tool_output\",\"message\":\"tool output survives\",\"details\":{\"exit_code\":2}}}\n",
                "{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/tmp/project\"}}\n",
                "{\"role\":\"assistant\",\"content\":{\"text\":\"object content survives\",\"extra\":{\"keep\":true}}}\n",
                "{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"uuid\":\"boundary\",\"content\":\"Compact boundary reached\"}\n",
                "{\"type\":\"user\",\"uuid\":\"summary\",\"message\":{\"role\":\"user\",\"content\":\"This session is being continued from a previous conversation that ran out of context. The previous conversation is summarized below:\\nSummary:\\nDECISION: keep data.\"}}\n",
                "{\"type\":\"broken\",\n"
            ),
        )
        .unwrap();

        let (turns, diag) = import_session(&session_path).unwrap();

        assert_eq!(diag.lines_read, 8);
        assert_eq!(
            diag.lines_read,
            diag.mapped_records + diag.intentionally_skipped_records + diag.malformed_records
        );
        assert!(diag.accounting_balanced);
        assert_eq!(diag.malformed_records, 1);
        assert_eq!(diag.intentionally_skipped_records, 1);
        assert!(diag.warnings.iter().any(|w| w.contains("Invalid JSON")));
        assert!(diag.warnings.iter().any(|w| w.contains("future_event")));
        assert!(diag.warnings.iter().any(|w| w.contains("tool_output")));
        assert!(turns.iter().any(|t| t.record_type == "compact_summary"));
        assert!(turns.iter().any(|t| t
            .content
            .as_deref()
            .unwrap_or("")
            .contains("object content survives")));
    }
}
