use anyhow::Context;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub struct ApifyConfig<'a> {
    pub token: &'a str,
    /// actor ID or task ID (actor takes precedence)
    pub actor_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub input_url: Option<&'a str>,
    /// evaluated provider name (from --source)
    pub provider: &'a str,
}

pub fn load_provider_health(
    health_source: &str,
    file_path: Option<&Path>,
) -> anyhow::Result<Value> {
    match health_source {
        "file" => load_from_file(file_path),
        "none" => Ok(stub_health("none", "mock")),
        _ => Err(anyhow::anyhow!(
            "Unsupported provider health source: {}",
            health_source
        )),
    }
}

/// Called when --provider-health apify is used.
/// Only reads the token here; never called from file/none paths.
pub fn load_provider_health_apify(
    cfg: &ApifyConfig<'_>,
    cache_file: Option<&Path>,
) -> (Value, Vec<String>) {
    let mut warnings = Vec::new();

    // If no token, skip live call entirely and go straight to cache/unknown
    let live_result = if cfg.token.is_empty() {
        warnings.push("APIFY_API_TOKEN not set; skipping live Apify call.".into());
        Err(anyhow::anyhow!("no token"))
    } else {
        fetch_apify(cfg)
    };

    match live_result {
        Ok(raw) => {
            let health = normalize_apify_response(&raw, cfg.provider, "apify");
            (health, warnings)
        }
        Err(e) => {
            if !e.to_string().contains("no token") {
                warnings.push(format!("Apify live call failed: {}", e));
            }
            // Try cache fallback
            if let Some(path) = cache_file {
                match load_from_file(Some(path)) {
                    Ok(mut cached) => {
                        if let Some(obj) = cached.as_object_mut() {
                            obj.insert("source".into(), json!("cached-apify"));
                        }
                        warnings.push(format!(
                            "Using cached provider health: {}",
                            path.display()
                        ));
                        return (cached, warnings);
                    }
                    Err(e2) => warnings.push(format!("Cache file load failed: {}", e2)),
                }
            }
            // No cache — return unknown stub
            warnings.push("Provider health status set to unknown.".into());
            (stub_health(cfg.provider, "apify"), warnings)
        }
    }
}

// ── private helpers ───────────────────────────────────────────────────────────

fn load_from_file(file_path: Option<&Path>) -> anyhow::Result<Value> {
    let path = file_path
        .ok_or_else(|| anyhow::anyhow!("Provider health file path required for 'file' source"))?;
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read provider health file: {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("Invalid JSON in provider health file: {}", path.display()))
}

fn stub_health(provider: &str, source: &str) -> Value {
    json!({
        "provider": provider,
        "status": "unknown",
        "confidence": 0.0,
        "reason": "No provider health information available.",
        "source": source,
        "source_url": null,
        "observed_at": chrono::Utc::now().to_rfc3339(),
    })
}

fn fetch_apify(cfg: &ApifyConfig<'_>) -> anyhow::Result<Value> {
    let client = reqwest::blocking::Client::new();

    // Prefer actor over task
    let (run_url, input_body) = if let Some(actor_id) = cfg.actor_id {
        (
            format!("https://api.apify.com/v2/acts/{}/runs", actor_id),
            build_input(cfg.input_url),
        )
    } else if let Some(task_id) = cfg.task_id {
        (
            format!("https://api.apify.com/v2/actor-tasks/{}/runs", task_id),
            build_input(cfg.input_url),
        )
    } else {
        anyhow::bail!("Either --apify-actor-id or --apify-task-id is required for apify mode");
    };

    // Start run (synchronous via ?waitForFinish)
    let run_url = format!("{}?waitForFinish=60", run_url);
    let resp = client
        .post(&run_url)
        .bearer_auth(cfg.token)
        .json(&input_body)
        .send()
        .context("Apify run request failed")?;

    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("Apify run failed ({}): {}", status, text);
    }

    let run: Value = serde_json::from_str(&text).context("Failed to parse Apify run response")?;
    let dataset_id = run
        .pointer("/data/defaultDatasetId")
        .and_then(|v| v.as_str())
        .context("Apify run response missing data.defaultDatasetId")?;

    // Fetch dataset items
    let items_url = format!(
        "https://api.apify.com/v2/datasets/{}/items?format=json",
        dataset_id
    );
    let items_resp = client
        .get(&items_url)
        .bearer_auth(cfg.token)
        .send()
        .context("Apify dataset fetch failed")?;

    let items_text = items_resp.text().unwrap_or_default();
    let items: Value =
        serde_json::from_str(&items_text).context("Failed to parse Apify dataset items")?;
    Ok(items)
}

fn build_input(url: Option<&str>) -> Value {
    match url {
        Some(u) => json!({ "startUrls": [{ "url": u }] }),
        None => json!({}),
    }
}

/// Best-effort normalization of an Apify response into our provider-health shape.
fn normalize_apify_response(raw: &Value, provider: &str, source: &str) -> Value {
    // raw may be an array of items or a single object
    let item = match raw {
        Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };

    let status = item
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let confidence = item
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let reason = item
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("Apify response received but no explicit status field was found.")
        .to_string();

    let source_url = item
        .get("source_url")
        .or_else(|| item.get("url"))
        .and_then(|v| v.as_str())
        .map(|s| json!(s))
        .unwrap_or(Value::Null);

    let observed_at = item
        .get("observed_at")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    json!({
        "provider": provider,
        "status": status,
        "confidence": confidence,
        "reason": reason,
        "source": source,
        "source_url": source_url,
        "observed_at": observed_at,
    })
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_provider_health_file_loading() {
        let path = Path::new("examples/provider-health/degraded.apify.cached.json");
        let health = load_provider_health("file", Some(path)).unwrap();
        assert_eq!(health["provider"].as_str(), Some("claude-code"));
        assert_eq!(health["status"].as_str(), Some("degraded"));
        assert!(health["reason"].as_str().unwrap().contains("Synthetic"));
    }

    #[test]
    fn test_provider_health_none_source() {
        let h = load_provider_health("none", None).unwrap();
        assert_eq!(h["status"].as_str(), Some("unknown"));
        assert_eq!(h["source"].as_str(), Some("mock"));
    }

    #[test]
    fn test_provider_health_invalid_source() {
        let result = load_provider_health("invalid", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported provider health source"));
    }

    #[test]
    fn test_normalize_apify_response_array() {
        let raw = json!([{
            "status": "degraded",
            "confidence": 0.8,
            "reason": "High latency",
            "source_url": "https://example.com",
            "observed_at": "2024-01-01T00:00:00Z"
        }]);
        let h = normalize_apify_response(&raw, "claude-code", "apify");
        assert_eq!(h["provider"], "claude-code");
        assert_eq!(h["status"], "degraded");
        assert_eq!(h["confidence"], 0.8);
        assert_eq!(h["source"], "apify");
        assert_eq!(h["source_url"], "https://example.com");
    }

    #[test]
    fn test_normalize_apify_response_missing_fields() {
        let raw = json!([{}]);
        let h = normalize_apify_response(&raw, "claude-code", "apify");
        assert_eq!(h["status"], "unknown");
        assert!(h["reason"].as_str().unwrap().contains("no explicit status field"));
    }

    #[test]
    fn test_apify_cache_fallback_no_token() {
        // No token + cache file → should succeed with source = "cached-apify"
        let cache_path = Path::new("examples/provider-health/degraded.apify.cached.json");
        let cfg = ApifyConfig {
            token: "",          // empty = no token
            actor_id: None,
            task_id: None,
            input_url: None,
            provider: "claude-code",
        };
        let (health, warnings) = load_provider_health_apify(&cfg, Some(cache_path));
        assert_eq!(health["source"].as_str(), Some("cached-apify"));
        assert_eq!(health["status"].as_str(), Some("degraded"));
        assert!(warnings.iter().any(|w| w.contains("APIFY_API_TOKEN not set")));
    }

    #[test]
    fn test_apify_no_token_no_cache_returns_unknown() {
        let cfg = ApifyConfig {
            token: "",
            actor_id: None,
            task_id: None,
            input_url: None,
            provider: "claude-code",
        };
        let (health, warnings) = load_provider_health_apify(&cfg, None);
        assert_eq!(health["status"].as_str(), Some("unknown"));
        assert!(warnings.iter().any(|w| w.contains("unknown")));
    }

    #[test]
    fn test_file_mode_does_not_use_apify_token() {
        let path = Path::new("examples/provider-health/degraded.apify.cached.json");
        let result = load_provider_health("file", Some(path));
        assert!(result.is_ok());
    }

    // Keep old name as alias so any external reference still compiles
    #[test]
    fn test_apify_cache_fallback() {
        test_apify_cache_fallback_no_token();
    }
}
