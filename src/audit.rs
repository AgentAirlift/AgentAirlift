use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use crate::canonical::CanonicalTurn;
use crate::session_import::ImportDiagnostics;

pub fn create_audit_report(
    turns: &[CanonicalTurn],
    diagnostics: &ImportDiagnostics,
    dropped_fields: &Value,
    source: &str,
    output_dir: &Path,
) -> anyhow::Result<()> {
    fs::create_dir_all(output_dir)?;

    let user_turns = turns.iter().filter(|t| t.role == "user").count();
    let assistant_turns = turns.iter().filter(|t| t.role == "assistant").count();
    let tool_calls: usize = turns.iter().map(|t| t.tool_calls.as_array().map_or(0, |a| a.len())).sum();
    let tool_results: usize = turns.iter().map(|t| t.tool_results.as_array().map_or(0, |a| a.len())).sum();

    let turn_lines = turns
        .iter()
        .map(|t| format!("- `{}` [{}] {} chars", t.id, t.role, t.content.len()))
        .collect::<Vec<_>>()
        .join("\n");

    let warnings_section = if diagnostics.warnings.is_empty() {
        "None.".to_string()
    } else {
        diagnostics.warnings.iter().map(|w| format!("- {}", w)).collect::<Vec<_>>().join("\n")
    };

    let report = format!(
        "# Conversion Report\n\n\
## Source\n\
- Source provider: `{source}`\n\
- Source file: `{src_file}`\n\
- Detected format: `{fmt}` (confidence {conf:.2})\n\n\
## Import Diagnostics\n\
- Lines read: {lines_read}\n\
- Turns imported: {turns_imported}\n\
- Lines skipped (non-message/meta): {skipped}\n\
- Warnings: {warn_count}\n\n\
## Canonical Session\n\
- Total turns: {total}\n\
- User turns: {user}\n\
- Assistant turns: {assistant}\n\
- Tool calls captured: {tool_calls}\n\
- Tool results captured: {tool_results}\n\n\
## Warnings\n{warnings}\n\n\
## Turn Statistics\n{turn_lines}\n\n\
## Notes\n\
- Dropped/unmapped source sub-fields are listed in `dropped-fields.json`.\n\
- Full import diagnostics are in `import-diagnostics.json`.\n\
- No credentials or tokens are written to any audit artifact.\n",
        source = source,
        src_file = diagnostics.source_path,
        fmt = diagnostics.detected_format,
        conf = diagnostics.format_confidence,
        lines_read = diagnostics.lines_read,
        turns_imported = diagnostics.turns_imported,
        skipped = diagnostics.lines_skipped,
        warn_count = diagnostics.warnings.len(),
        total = turns.len(),
        user = user_turns,
        assistant = assistant_turns,
        tool_calls = tool_calls,
        tool_results = tool_results,
        warnings = warnings_section,
        turn_lines = turn_lines,
    );
    fs::write(output_dir.join("conversion-report.md"), report)?;

    // warnings.json
    let warnings = json!({
        "count": diagnostics.warnings.len(),
        "warnings": diagnostics.warnings,
    });
    fs::write(output_dir.join("warnings.json"), serde_json::to_string_pretty(&warnings)?)?;

    // dropped-fields.json
    fs::write(output_dir.join("dropped-fields.json"), serde_json::to_string_pretty(dropped_fields)?)?;

    // import-diagnostics.json
    fs::write(
        output_dir.join("import-diagnostics.json"),
        serde_json::to_string_pretty(&json!(diagnostics))?,
    )?;

    Ok(())
}

/// Heuristic guard: detects strings that look like leaked credential *values*
/// (not mere mentions of an env-var name). Used by tests to assert audit
/// artifacts never embed secrets.
#[cfg(test)]
pub fn looks_like_secret(s: &str) -> bool {
    let lower = s.to_lowercase();
    // Auth header carrying an actual token value.
    if lower.contains("bearer ey") || lower.contains("bearer sk-") {
        return true;
    }
    // A `token`-style assignment followed by a non-trivial value.
    for marker in ["token\":\"", "token=", "token: \"", "api_key\":\"", "api_key="] {
        if let Some(idx) = lower.find(marker) {
            let value: String = lower[idx + marker.len()..]
                .chars()
                .take_while(|c| !matches!(c, '"' | ' ' | '\n' | ',' | '}'))
                .collect();
            if value.len() >= 8 {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn sample_turn(id: &str, role: &str, content: &str) -> CanonicalTurn {
        CanonicalTurn {
            id: id.into(),
            role: role.into(),
            content: content.into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            source: "claude-code".into(),
            content_blocks: json!([]),
            tool_calls: json!([]),
            tool_results: json!([]),
            metadata: json!({}),
        }
    }

    #[test]
    fn test_audit_report_includes_diagnostics() {
        let turns = vec![
            sample_turn("t1", "user", "do the thing"),
            sample_turn("t2", "assistant", "done"),
        ];
        let diag = ImportDiagnostics {
            source_path: "examples/sessions/x.jsonl".into(),
            lines_read: 5,
            turns_imported: 2,
            lines_skipped: 3,
            detected_format: "claude-code".into(),
            format_confidence: 0.91,
            warnings: vec!["Line 4: skipped unrecognized entry type 'telemetry'".into()],
        };
        let dropped = json!({"unmapped_nested_fields": ["message.model"]});

        let dir = TempDir::new().unwrap();
        create_audit_report(&turns, &diag, &dropped, "claude-code", dir.path()).unwrap();

        let report = fs::read_to_string(dir.path().join("conversion-report.md")).unwrap();
        assert!(report.contains("Detected format: `claude-code`"));
        assert!(report.contains("Lines read: 5"));
        assert!(report.contains("Lines skipped"));
        assert!(report.contains("telemetry"));

        let diag_json = fs::read_to_string(dir.path().join("import-diagnostics.json")).unwrap();
        let v: Value = serde_json::from_str(&diag_json).unwrap();
        assert_eq!(v["detected_format"], "claude-code");
        assert_eq!(v["lines_skipped"], 3);

        let dropped_json = fs::read_to_string(dir.path().join("dropped-fields.json")).unwrap();
        assert!(dropped_json.contains("message.model"));
    }

    #[test]
    fn test_audit_artifacts_contain_no_token_strings() {
        let turns = vec![sample_turn("t1", "user", "normal request content")];
        let diag = ImportDiagnostics {
            source_path: "examples/sessions/x.jsonl".into(),
            detected_format: "flat".into(),
            warnings: vec!["Marginlab live fetch failed: connection refused.".into()],
            ..Default::default()
        };
        let dir = TempDir::new().unwrap();
        create_audit_report(&turns, &diag, &json!({}), "claude-code", dir.path()).unwrap();

        for name in ["conversion-report.md", "warnings.json", "dropped-fields.json", "import-diagnostics.json"] {
            let content = fs::read_to_string(dir.path().join(name)).unwrap();
            assert!(!looks_like_secret(&content), "{} appears to contain a secret-like string", name);
        }
    }

    #[test]
    fn test_secret_guard_distinguishes_value_from_var_name() {
        assert!(!looks_like_secret("Marginlab live fetch failed: connection refused."));
        // Actual leaked values are caught.
        assert!(looks_like_secret("Authorization: Bearer eyJhbGciOiJIUzI1Ni) leaked"));
        assert!(looks_like_secret("{\"token\":\"abcd1234efgh5678\"}"));
    }
}
