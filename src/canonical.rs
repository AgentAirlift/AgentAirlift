use serde_json::{json, Value};
use crate::session_import::SessionTurn;

#[derive(Debug, serde::Serialize)]
pub struct CanonicalTurn {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub metadata: Value,
}

pub fn normalize_turns(turns: Vec<SessionTurn>) -> (Vec<CanonicalTurn>, Vec<String>, Value) {
    let mut canonical_turns = Vec::new();
    let warnings = Vec::new();
    let dropped_fields = json!({});
    
    for (i, turn) in turns.into_iter().enumerate() {
        let id = turn.id.unwrap_or_else(|| format!("turn-{}", i + 1));
        let role = turn.role.unwrap_or_else(|| "unknown".to_string());
        let content = turn.content.unwrap_or_else(|| "".to_string());
        
        // Extract timestamp from raw JSON or use current time
        let timestamp = turn.raw.get("timestamp")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        
        // Collect metadata from remaining fields
        let mut metadata = json!({});
        if let Value::Object(obj) = turn.raw {
            for (key, value) in obj {
                match key.as_str() {
                    "id" | "role" | "content" | "timestamp" => continue,
                    _ => {
                        metadata[key] = value;
                    }
                }
            }
        }
        
        canonical_turns.push(CanonicalTurn {
            id,
            role,
            content,
            timestamp,
            metadata,
        });
    }
    
    (canonical_turns, warnings, dropped_fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_canonical_session_generation() {
        // Test 4: canonical session generation preserves turn count
        // Create test session turns
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
            },
        ];
        
        let (canonical_turns, warnings, dropped_fields) = normalize_turns(turns);
        
        assert_eq!(canonical_turns.len(), 2, "Should preserve all 2 turns");
        assert!(warnings.is_empty(), "Should have no warnings for valid turns");
        assert!(dropped_fields.is_object(), "Dropped fields should be a JSON object");
        
        // Verify canonical structure
        let first_canonical = &canonical_turns[0];
        assert_eq!(first_canonical.id, "test-1");
        assert_eq!(first_canonical.role, "user");
        assert_eq!(first_canonical.content, "Test content 1");
        assert_eq!(first_canonical.timestamp, "2024-01-01T10:00:00Z");
        
        // Verify metadata preservation
        assert_eq!(first_canonical.metadata["extra_field"], "extra_value");
        
        // Verify second turn
        let second_canonical = &canonical_turns[1];
        assert_eq!(second_canonical.id, "test-2");
        assert_eq!(second_canonical.role, "assistant");
        assert_eq!(second_canonical.content, "Test response 1");
    }

    #[test]
    fn test_canonical_with_missing_fields() {
        // Test canonical generation with missing fields
        let turns = vec![
            SessionTurn {
                raw: json!({
                    "content": "Test without id or role"
                }),
                id: None,
                role: None,
                content: Some("Test without id or role".to_string()),
            },
        ];
        
        let (canonical_turns, warnings, _) = normalize_turns(turns);
        
        assert_eq!(canonical_turns.len(), 1);
        assert!(warnings.is_empty());
        
        let canonical = &canonical_turns[0];
        assert_eq!(canonical.id, "turn-1"); // Generated ID
        assert_eq!(canonical.role, "unknown"); // Default role
        assert_eq!(canonical.content, "Test without id or role");
        assert!(!canonical.timestamp.is_empty()); // Should have generated timestamp
    }
}