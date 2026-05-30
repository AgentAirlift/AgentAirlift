use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};
use anyhow::Context;

#[derive(Debug)]
pub struct SessionTurn {
    pub raw: Value,
    pub id: Option<String>,
    pub role: Option<String>,
    pub content: Option<String>,
}

pub fn import_session(path: &std::path::Path) -> anyhow::Result<(Vec<SessionTurn>, Vec<String>)> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open session file: {}", path.display()))?;
    
    let reader = BufReader::new(file);
    let mut turns = Vec::new();
    let mut warnings = Vec::new();
    
    for (line_num, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("Failed to read line {}", line_num + 1))?;
        
        if line.trim().is_empty() {
            continue;
        }
        
        match serde_json::from_str::<Value>(&line) {
            Ok(raw) => {
                let turn = SessionTurn {
                    id: raw.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    role: raw.get("role").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    content: raw.get("content").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    raw,
                };
                turns.push(turn);
            }
            Err(e) => {
                warnings.push(format!("Line {}: Invalid JSON - {}", line_num + 1, e));
            }
        }
    }
    
    if turns.is_empty() && warnings.is_empty() {
        return Err(anyhow::anyhow!("Session file is empty or contains no valid JSONL rows"));
    }
    
    Ok((turns, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_import_session_parses_expected_turns() {
        // Test 1: sample-claude-like.jsonl parses into expected number of turns
        let path = std::path::Path::new("examples/sessions/sample-claude-like.jsonl");
        let (turns, warnings) = import_session(path).unwrap();
        
        assert_eq!(turns.len(), 6, "Should parse exactly 6 turns from sample session");
        assert!(warnings.is_empty(), "Should have no warnings for valid JSONL");
        
        // Verify first turn content
        let first_turn = &turns[0];
        assert_eq!(first_turn.id.as_deref(), Some("turn-1"));
        assert_eq!(first_turn.role.as_deref(), Some("user"));
        assert!(first_turn.content.as_deref().unwrap().contains("Create a simple Rust CLI"));
    }

    #[test]
    fn test_invalid_jsonl_produces_warnings_not_panics() {
        // Test 2: invalid JSONL rows produce warnings instead of panics
        let temp_dir = TempDir::new().unwrap();
        let invalid_jsonl_path = temp_dir.path().join("invalid.jsonl");
        
        // Create a file with both valid and invalid JSONL
        fs::write(&invalid_jsonl_path, r#"{"id": "valid-1", "role": "user", "content": "test"}
invalid json here
{"id": "valid-2", "role": "assistant", "content": "response"}"#).unwrap();
        
        let (turns, warnings) = import_session(&invalid_jsonl_path).unwrap();
        
        assert_eq!(turns.len(), 2, "Should parse 2 valid turns");
        assert_eq!(warnings.len(), 1, "Should have 1 warning for invalid line");
        assert!(warnings[0].contains("Invalid JSON"), "Warning should mention invalid JSON");
        
        // Verify we didn't panic and got the valid turns
        assert_eq!(turns[0].id.as_deref(), Some("valid-1"));
        assert_eq!(turns[1].id.as_deref(), Some("valid-2"));
    }
}