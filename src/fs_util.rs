use std::fs;
use std::path::Path;

#[allow(dead_code)]
pub fn copy_file_with_dir(src: &Path, dst: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    Ok(())
}

pub fn write_json_pretty(path: &Path, value: &serde_json::Value) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(value)?;
    fs::write(path, content)?;
    Ok(())
}

pub fn write_jsonl(path: &Path, lines: &[String]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = lines.join("\n");
    fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_replay_jsonl_generation() {
        // Test 5: replay JSONL generation emits at least one message row
        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path();
        
        // Create test replay lines
        let replay_lines = vec![
            serde_json::json!({
                "id": "turn-1",
                "role": "user",
                "content": "Test content",
                "timestamp": "2024-01-01T10:00:00Z"
            }).to_string(),
            serde_json::json!({
                "id": "turn-2",
                "role": "assistant",
                "content": "Test response",
                "timestamp": "2024-01-01T10:00:05Z"
            }).to_string(),
        ];
        
        // Generate replay JSONL
        write_jsonl(&output_dir.join("replay.session.jsonl"), &replay_lines).unwrap();
        
        // Verify file was created and has content
        let replay_content = fs::read_to_string(output_dir.join("replay.session.jsonl")).unwrap();
        assert!(!replay_content.is_empty(), "Replay JSONL should not be empty");
        
        // Count lines (should be 2)
        let line_count = replay_content.lines().count();
        assert_eq!(line_count, 2, "Should have 2 lines in replay JSONL");
        
        // Verify first line is valid JSON
        let first_line = replay_content.lines().next().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(first_line).unwrap();
        assert_eq!(parsed["id"].as_str(), Some("turn-1"));
        assert_eq!(parsed["role"].as_str(), Some("user"));
    }

    #[test]
    fn test_write_json_pretty() {
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("test.json");
        
        let test_value = serde_json::json!({
            "test": "value",
            "number": 42
        });
        
        write_json_pretty(&output_path, &test_value).unwrap();
        
        let content = fs::read_to_string(&output_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["test"].as_str(), Some("value"));
        assert_eq!(parsed["number"].as_i64(), Some(42));
    }
}