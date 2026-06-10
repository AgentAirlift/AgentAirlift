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
    if !diagnostics.accounting_balanced {
        anyhow::bail!(
            "Import accounting mismatch: lines_read={} mapped={} skipped={} malformed={}",
            diagnostics.lines_read,
            diagnostics.mapped_records,
            diagnostics.intentionally_skipped_records,
            diagnostics.malformed_records,
        );
    }
    match dropped_fields.get("unapproved_drops").and_then(|v| v.as_array()) {
        Some(drops) if drops.is_empty() => {}
        Some(_) => {
            anyhow::bail!("Unapproved dropped fields detected: {}", dropped_fields["unapproved_drops"]);
        }
        None => anyhow::bail!("Dropped-field budget missing `unapproved_drops` array"),
    }

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
- Mapped records: {mapped}\n\
- Intentionally skipped records: {intentional_skips}\n\
- Malformed records: {malformed}\n\
- Preserved unknown records: {unknown}\n\
- Accounting balanced: {balanced}\n\
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
        mapped = diagnostics.mapped_records,
        intentional_skips = diagnostics.intentionally_skipped_records,
        malformed = diagnostics.malformed_records,
        unknown = diagnostics.preserved_unknown_records,
        balanced = diagnostics.accounting_balanced,
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

pub fn create_ci_gate_report(
    turns: &[CanonicalTurn],
    diagnostics: &ImportDiagnostics,
    dropped_fields: &Value,
    targets: &[String],
    output_root: &Path,
) -> anyhow::Result<()> {
    let audit_dir = output_root.join("audit");
    fs::create_dir_all(&audit_dir)?;

    let accounting_balanced = diagnostics.accounting_balanced
        && diagnostics.lines_read
            == diagnostics.mapped_records
                + diagnostics.intentionally_skipped_records
                + diagnostics.malformed_records;
    let dropped_field_budget_clean = dropped_fields
        .get("unapproved_drops")
        .and_then(|v| v.as_array())
        .is_some_and(|drops| drops.is_empty());
    let canonical_nonempty = !turns.is_empty()
        && read_json_file(&output_root.join("normalized/canonical-session.json"))
            .ok()
            .and_then(|v| v.as_array().map(|arr| arr.len() == turns.len()))
            .unwrap_or(false);
    let hashes_present = turns.iter().all(|turn| {
        is_sha256_hex(&turn.raw_sha256) && is_sha256_hex(&turn.canonical_sha256)
    });
    let replay_hashes_match = replay_hashes_match(output_root, turns).unwrap_or(false);
    let exports_nonempty = exports_nonempty(output_root, targets);
    let exports_match_canonical = exports_match_canonical(output_root, targets, turns).unwrap_or(false);

    let checks = json!({
        "accounting_balanced": accounting_balanced,
        "dropped_field_budget_clean": dropped_field_budget_clean,
        "canonical_nonempty": canonical_nonempty,
        "hashes_present": hashes_present,
        "replay_hashes_match": replay_hashes_match,
        "exports_nonempty": exports_nonempty,
        "exports_match_canonical": exports_match_canonical,
    });

    let failures = checks
        .as_object()
        .unwrap()
        .iter()
        .filter_map(|(name, passed)| {
            if passed.as_bool() == Some(true) {
                None
            } else {
                Some(name.clone())
            }
        })
        .collect::<Vec<_>>();
    let report = json!({
        "passed": failures.is_empty(),
        "checks": checks,
        "failures": failures,
        "targets": targets,
        "canonical_turns": turns.len(),
    });
    fs::write(
        audit_dir.join("ci-gate.json"),
        serde_json::to_string_pretty(&report)?,
    )?;

    if !report["passed"].as_bool().unwrap_or(false) {
        anyhow::bail!("CI gate failed: {}", report["failures"]);
    }
    Ok(())
}

fn read_json_file(path: &Path) -> anyhow::Result<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn read_jsonl_file(path: &Path) -> anyhow::Result<Vec<Value>> {
    Ok(fs::read_to_string(path)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<_>, _>>()?)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn replay_hashes_match(output_root: &Path, turns: &[CanonicalTurn]) -> anyhow::Result<bool> {
    let replay = read_jsonl_file(&output_root.join("replay/agent-airlift.session.jsonl"))?;
    if replay.len() != turns.len() {
        return Ok(false);
    }
    Ok(replay.iter().zip(turns.iter()).all(|(line, turn)| {
        line["canonical_sha256"] == turn.canonical_sha256
            && line["canonical"]["canonical_sha256"] == turn.canonical_sha256
    }))
}

fn exports_nonempty(output_root: &Path, targets: &[String]) -> bool {
    let exports_dir = output_root.join("exports");
    if !has_nonempty_file(&exports_dir.join("HANDOFF.md"))
        || !has_nonempty_file(&exports_dir.join("AGENTS.md"))
    {
        return false;
    }
    targets.iter().all(|target| match target.as_str() {
        "codex" => {
            has_nonempty_file(&exports_dir.join("codex-like.session.jsonl"))
                && has_any_file(&exports_dir.join("native/codex"))
        }
        "claude-code" => {
            has_nonempty_file(&exports_dir.join("claude-code-like.session.jsonl"))
                && has_any_file(&exports_dir.join("native/claude-code"))
        }
        "kiro" => has_nonempty_file(&exports_dir.join("kiro-session.json")),
        "opencode" => has_nonempty_file(&exports_dir.join("opencode-like.session.jsonl")),
        _ => false,
    })
}

fn has_nonempty_file(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

fn has_any_file(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| entry.path().is_file())
        })
        .unwrap_or(false)
}

fn exports_match_canonical(
    output_root: &Path,
    targets: &[String],
    turns: &[CanonicalTurn],
) -> anyhow::Result<bool> {
    let expected = turns
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    for target in targets {
        let actual = export_canonical_sidecars(output_root, target)?;
        if actual != expected {
            return Ok(false);
        }
    }
    Ok(true)
}

fn export_canonical_sidecars(output_root: &Path, target: &str) -> anyhow::Result<Vec<Value>> {
    let exports_dir = output_root.join("exports");
    match target {
        "codex" => jsonl_canonical_sidecars(&exports_dir.join("codex-like.session.jsonl")),
        "claude-code" => jsonl_canonical_sidecars(&exports_dir.join("claude-code-like.session.jsonl")),
        "opencode" => jsonl_canonical_sidecars(&exports_dir.join("opencode-like.session.jsonl")),
        "kiro" => Ok(read_json_file(&exports_dir.join("kiro-session.json"))?["turns"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|turn| turn["agent_airlift_canonical"].clone())
            .collect()),
        _ => anyhow::bail!("Unsupported target in CI gate: {}", target),
    }
}

fn jsonl_canonical_sidecars(path: &Path) -> anyhow::Result<Vec<Value>> {
    Ok(read_jsonl_file(path)?
        .into_iter()
        .skip(1)
        .map(|line| line["agent_airlift_canonical"].clone())
        .collect())
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
            ..Default::default()
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
            lines_skipped: 2,
            mapped_records: 2,
            intentionally_skipped_records: 2,
            malformed_records: 1,
            preserved_unknown_records: 1,
            accounting_balanced: true,
            detected_format: "claude-code".into(),
            format_confidence: 0.91,
            warnings: vec!["Line 4: skipped unrecognized entry type 'telemetry'".into()],
            ..Default::default()
        };
        let dropped = json!({"unapproved_drops": [], "preserved_nested_fields": ["message.model"]});

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
        assert_eq!(v["lines_skipped"], 2);

        let dropped_json = fs::read_to_string(dir.path().join("dropped-fields.json")).unwrap();
        assert!(dropped_json.contains("message.model"));
    }

    #[test]
    fn test_audit_artifacts_contain_no_token_strings() {
        let turns = vec![sample_turn("t1", "user", "normal request content")];
        let diag = ImportDiagnostics {
            source_path: "examples/sessions/x.jsonl".into(),
            detected_format: "flat".into(),
            accounting_balanced: true,
            warnings: vec!["Marginlab live fetch failed: connection refused.".into()],
            ..Default::default()
        };
        let dir = TempDir::new().unwrap();
        create_audit_report(&turns, &diag, &json!({"unapproved_drops": []}), "claude-code", dir.path()).unwrap();

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

    #[test]
    fn test_audit_rejects_missing_drop_budget() {
        let turns = vec![sample_turn("t1", "user", "normal request content")];
        let diag = ImportDiagnostics {
            accounting_balanced: true,
            ..Default::default()
        };
        let dir = TempDir::new().unwrap();
        let err = create_audit_report(&turns, &diag, &json!({}), "claude-code", dir.path()).unwrap_err();
        assert!(err.to_string().contains("unapproved_drops"));
    }

    #[test]
    fn test_audit_rejects_unapproved_drops() {
        let turns = vec![sample_turn("t1", "user", "normal request content")];
        let diag = ImportDiagnostics {
            accounting_balanced: true,
            ..Default::default()
        };
        let dir = TempDir::new().unwrap();
        let err = create_audit_report(
            &turns,
            &diag,
            &json!({"unapproved_drops": [{"field": "message.secret"}]}),
            "claude-code",
            dir.path(),
        ).unwrap_err();
        assert!(err.to_string().contains("Unapproved dropped fields"));
    }
}
