use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use anyhow::Context;

/// A single imported turn with structured extras preserved where available.
/// `raw` always retains the original JSON line so no information is lost.
#[derive(Debug, Default)]
pub struct SessionTurn {
    pub raw: Value,
    pub id: Option<String>,
    pub role: Option<String>,
    pub content: Option<String>,
    pub timestamp: Option<String>,
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

/// Non-message Claude Code entry types that are intentionally skipped.
const CLAUDE_SKIP_TYPES: &[&str] = &["progress", "system", "file-history-snapshot", "summary"];

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
                diag.warnings.push(format!("Line {}: Invalid JSON - {}", line_num, e));
                continue;
            }
        };

        match classify_and_extract(raw, line_num) {
            LineOutcome::Turn(turn, kind) => {
                tally(kind, &mut claude, &mut codex, &mut flat, &mut unknown);
                turns.push(turn);
                diag.turns_imported += 1;
            }
            LineOutcome::Skip(kind, warning) => {
                tally(kind, &mut claude, &mut codex, &mut flat, &mut unknown);
                diag.lines_skipped += 1;
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

    if turns.is_empty() && diag.warnings.is_empty() {
        return Err(anyhow::anyhow!("Session file is empty or contains no valid JSONL rows"));
    }

    Ok((turns, diag))
}

enum LineOutcome {
    Turn(SessionTurn, FormatKind),
    Skip(FormatKind, Option<String>),
}

fn tally(kind: FormatKind, claude: &mut usize, codex: &mut usize, flat: &mut usize, unknown: &mut usize) {
    match kind {
        FormatKind::ClaudeCode => *claude += 1,
        FormatKind::Codex => *codex += 1,
        FormatKind::Flat => *flat += 1,
        FormatKind::Unknown => *unknown += 1,
    }
}

/// Routes a parsed line to the right extractor based on shape.
fn classify_and_extract(raw: Value, line_num: usize) -> LineOutcome {
    let entry_type = raw.get("type").and_then(|v| v.as_str()).map(str::to_string);
    if let Some(ty) = entry_type {
        match ty.as_str() {
            "user" | "assistant" => return extract_claude_message(raw, &ty),
            "event_msg" => return extract_codex_event(raw, line_num),
            "response_item" | "session_meta" | "turn_context" => {
                // response_item duplicates event_msg; meta carries no turn content.
                return LineOutcome::Skip(FormatKind::Codex, None);
            }
            t if CLAUDE_SKIP_TYPES.contains(&t) => {
                return LineOutcome::Skip(FormatKind::ClaudeCode, None);
            }
            other => {
                return LineOutcome::Skip(
                    FormatKind::Unknown,
                    Some(format!("Line {}: skipped unrecognized entry type '{}'", line_num, other)),
                );
            }
        }
    }
    // No `type` field → flat {id, role, content} shape
    if raw.get("role").is_some() || raw.get("content").is_some() {
        return extract_flat(raw);
    }
    LineOutcome::Skip(
        FormatKind::Unknown,
        Some(format!("Line {}: skipped row with no recognizable role/content/type", line_num)),
    )
}

/// Flat `{id, role, content, timestamp, ...}` rows (our demo + simple exports).
fn extract_flat(raw: Value) -> LineOutcome {
    let turn = SessionTurn {
        id: raw.get("id").and_then(|v| v.as_str()).map(str::to_string),
        role: raw.get("role").and_then(|v| v.as_str()).map(str::to_string),
        content: raw.get("content").and_then(|v| v.as_str()).map(str::to_string),
        timestamp: raw.get("timestamp").and_then(|v| v.as_str()).map(str::to_string),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        raw,
    };
    LineOutcome::Turn(turn, FormatKind::Flat)
}

/// Claude Code entries: `message.content` is a string or an array of typed blocks.
fn extract_claude_message(raw: Value, ty: &str) -> LineOutcome {
    let message = raw.get("message").cloned().unwrap_or(Value::Null);
    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or(ty)
        .to_string();

    let mut texts: Vec<String> = Vec::new();
    let mut tool_calls = Vec::new();
    let mut tool_results = Vec::new();

    match message.get("content") {
        Some(Value::String(s)) => texts.push(s.clone()),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            texts.push(t.to_string());
                        }
                    }
                    Some("tool_use") => {
                        tool_calls.push(json!({
                            "id": block.get("id").cloned().unwrap_or(Value::Null),
                            "name": block.get("name").cloned().unwrap_or(Value::Null),
                            "input": block.get("input").cloned().unwrap_or(Value::Null),
                        }));
                    }
                    Some("tool_result") => {
                        let result_text = stringify_tool_result(block.get("content"));
                        let is_error = block.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                        tool_results.push(json!({
                            "tool_use_id": block.get("tool_use_id").cloned().unwrap_or(Value::Null),
                            "content": result_text,
                            "is_error": is_error,
                        }));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    let content = synthesize_content(&texts, &tool_calls, &tool_results);
    let turn = SessionTurn {
        id: raw.get("uuid").or_else(|| raw.get("id")).and_then(|v| v.as_str()).map(str::to_string),
        role: Some(role),
        content: Some(content),
        timestamp: raw.get("timestamp").and_then(|v| v.as_str()).map(str::to_string),
        tool_calls,
        tool_results,
        raw,
    };
    LineOutcome::Turn(turn, FormatKind::ClaudeCode)
}

/// Codex `event_msg` entries carry user/agent messages under `payload`.
fn extract_codex_event(raw: Value, line_num: usize) -> LineOutcome {
    let payload = raw.get("payload").cloned().unwrap_or(Value::Null);
    let ptype = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let role = match ptype {
        "user_message" => "user",
        "agent_message" => "assistant",
        other => {
            return LineOutcome::Skip(
                FormatKind::Codex,
                Some(format!("Line {}: skipped codex event_msg payload type '{}'", line_num, other)),
            );
        }
    };
    let content = payload.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let turn = SessionTurn {
        id: payload.get("id").and_then(|v| v.as_str()).map(str::to_string),
        role: Some(role.to_string()),
        content: Some(content),
        timestamp: raw.get("timestamp").and_then(|v| v.as_str()).map(str::to_string),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
        raw,
    };
    LineOutcome::Turn(turn, FormatKind::Codex)
}

/// tool_result content may be a string or an array of `{type:text,text}` blocks.
fn stringify_tool_result(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|v| v.as_str()).map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
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
                let body = r.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let err = r.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                let label = if err { "[tool result: error]" } else { "[tool result]" };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_import_session_parses_expected_turns() {
        let path = Path::new("examples/sessions/sample-claude-like.jsonl");
        let (turns, diag) = import_session(path).unwrap();
        assert_eq!(turns.len(), 6, "Should parse exactly 6 turns from sample session");
        assert!(diag.warnings.is_empty(), "Should have no warnings for valid JSONL");
        assert_eq!(diag.detected_format, "flat");
        let first_turn = &turns[0];
        assert_eq!(first_turn.id.as_deref(), Some("turn-1"));
        assert_eq!(first_turn.role.as_deref(), Some("user"));
        assert!(first_turn.content.as_deref().unwrap().contains("Create a simple Rust CLI"));
    }

    #[test]
    fn test_invalid_jsonl_produces_warnings_not_panics() {
        let temp_dir = TempDir::new().unwrap();
        let invalid_jsonl_path = temp_dir.path().join("invalid.jsonl");
        fs::write(&invalid_jsonl_path, "{\"id\": \"valid-1\", \"role\": \"user\", \"content\": \"test\"}\ninvalid json here\n{\"id\": \"valid-2\", \"role\": \"assistant\", \"content\": \"response\"}").unwrap();

        let (turns, diag) = import_session(&invalid_jsonl_path).unwrap();
        assert_eq!(turns.len(), 2, "Should parse 2 valid turns");
        assert_eq!(diag.warnings.len(), 1, "Should have 1 warning for invalid line");
        assert!(diag.warnings[0].contains("Invalid JSON"));
        assert_eq!(turns[0].id.as_deref(), Some("valid-1"));
        assert_eq!(turns[1].id.as_deref(), Some("valid-2"));
    }

    #[test]
    fn test_claude_code_nested_format_with_tool_calls() {
        let path = Path::new("examples/sessions/claude-code-realistic.jsonl");
        let (turns, diag) = import_session(path).unwrap();
        assert_eq!(diag.detected_format, "claude-code");
        assert!(diag.format_confidence > 0.9, "confidence should be high, got {}", diag.format_confidence);
        // file-history-snapshot + progress are skipped, not turned into turns
        assert!(diag.lines_skipped >= 2);
        // 9 message turns: 5 user (incl. tool_result turns) + 4 assistant
        assert_eq!(turns.len(), 9, "got {} turns", turns.len());

        // The first assistant turn used a Read tool
        let assistant_with_tool = turns.iter().find(|t| !t.tool_calls.is_empty()).unwrap();
        assert_eq!(assistant_with_tool.tool_calls[0]["name"], "Read");

        // There is an error tool_result somewhere
        let has_error_result = turns.iter().any(|t| {
            t.tool_results.iter().any(|r| r["is_error"].as_bool() == Some(true))
        });
        assert!(has_error_result, "should capture an is_error tool_result");
    }

    #[test]
    fn test_codex_event_msg_format() {
        let path = Path::new("examples/sessions/codex-realistic.jsonl");
        let (turns, diag) = import_session(path).unwrap();
        assert_eq!(diag.detected_format, "codex");
        // session_meta + response_item duplicate are skipped
        assert!(diag.lines_skipped >= 2);
        // user + agent + user(command output) + agent = 4 event_msg turns
        assert_eq!(turns.len(), 4, "got {} turns", turns.len());
        assert_eq!(turns[0].role.as_deref(), Some("user"));
        assert_eq!(turns[1].role.as_deref(), Some("assistant"));
    }

    #[test]
    fn test_edge_cases_tolerated_with_structured_warnings() {
        let path = Path::new("examples/sessions/edge-cases.jsonl");
        let (turns, diag) = import_session(path).unwrap();
        // e-1, e-2, e-3 are valid flat turns
        assert_eq!(turns.len(), 3, "got {} turns", turns.len());
        // unknown fields preserved in raw
        assert_eq!(turns[0].raw["unmapped_top"], "keep-me");
        assert_eq!(turns[0].raw["custom_meta"]["client"], "cli");
        // 2 invalid-json lines + 1 unknown-type telemetry warning = 3 warnings
        assert!(diag.warnings.iter().filter(|w| w.contains("Invalid JSON")).count() == 2);
        assert!(diag.warnings.iter().any(|w| w.contains("telemetry")));
    }
}
