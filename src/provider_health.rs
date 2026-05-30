use anyhow::Context;
use serde_json::Value;
use std::fs;

pub fn load_provider_health(health_source: &str, file_path: Option<&std::path::Path>) -> anyhow::Result<Value> {
    match health_source {
        "file" => {
            let path = file_path.ok_or_else(|| anyhow::anyhow!("Provider health file path required for 'file' source"))?;
            let content = fs::read_to_string(path)
                .with_context(|| format!("Failed to read provider health file: {}", path.display()))?;
            let health: Value = serde_json::from_str(&content)
                .with_context(|| format!("Invalid JSON in provider health file: {}", path.display()))?;
            Ok(health)
        }
        "none" => {
            Ok(serde_json::json!({
                "provider": "none",
                "status": "unknown",
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "message": "No provider health information available",
            }))
        }
        _ => {
            Err(anyhow::anyhow!("Unsupported provider health source: {}", health_source))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_health_file_loading() {
        // Test 3: provider-health file loading works and reads degraded status
        let path = std::path::Path::new("examples/provider-health/degraded.apify.cached.json");
        let health = load_provider_health("file", Some(path)).unwrap();
        
        // Verify the health data structure
        assert_eq!(health["provider"].as_str(), Some("apify"));
        assert_eq!(health["status"].as_str(), Some("degraded"));
        assert!(health["message"].as_str().unwrap().contains("High latency"));
        assert!(health["timestamp"].is_string());
    }

    #[test]
    fn test_provider_health_none_source() {
        // Test "none" provider health source
        let none_health = load_provider_health("none", None).unwrap();
        assert_eq!(none_health["provider"].as_str(), Some("none"));
        assert_eq!(none_health["status"].as_str(), Some("unknown"));
        assert!(none_health["message"].as_str().unwrap().contains("No provider health"));
    }

    #[test]
    fn test_provider_health_invalid_source() {
        // Test invalid provider health source
        let result = load_provider_health("invalid", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported provider health source"));
    }
}