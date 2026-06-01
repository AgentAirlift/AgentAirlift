use anyhow::Context;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub struct MarginlabConfig<'a> {
    /// Evaluated provider name from --source.
    pub provider: &'a str,
    /// Direct Marginlab tracker page URL.
    pub tracker_url: &'a str,
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

/// Called when --provider-health marginlab is used.
/// Returns (normalized_health, raw_marginlab_html_if_any, warnings).
pub fn load_provider_health_marginlab(
    cfg: &MarginlabConfig<'_>,
    cache_file: Option<&Path>,
) -> (Value, Option<String>, Vec<String>) {
    let mut warnings = Vec::new();

    match fetch_marginlab(cfg.tracker_url) {
        Ok(body) => {
            let health = normalize_marginlab_response(&body, cfg.provider, cfg.tracker_url);
            if should_use_cache_for_unrecognized_marginlab(&health) {
                if let Some(path) = cache_file {
                    if let Ok(cached) = load_cached_health(path, "cached-marginlab") {
                        warnings.push(
                            "Marginlab response was received but no clear status was found; \
                             using cached provider-health fixture.".into(),
                        );
                        return (cached, Some(body), warnings);
                    }
                }
            }
            (health, Some(body), warnings)
        }
        Err(e) => {
            warnings.push(format!("Marginlab live fetch failed: {}", e));
            if let Some(path) = cache_file {
                match load_cached_health(path, "cached-marginlab") {
                    Ok(cached) => {
                        warnings.push(format!("Using cached provider health: {}", path.display()));
                        return (cached, None, warnings);
                    }
                    Err(e2) => warnings.push(format!("Cache file load failed: {}", e2)),
                }
            }
            warnings.push("Provider health status set to unknown.".into());
            (stub_health(cfg.provider, "marginlab"), None, warnings)
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

fn load_cached_health(path: &Path, source: &str) -> anyhow::Result<Value> {
    let mut cached = load_from_file(Some(path))?;
    if let Some(obj) = cached.as_object_mut() {
        obj.insert("source".into(), json!(source));
    }
    Ok(cached)
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

fn fetch_marginlab(tracker_url: &str) -> anyhow::Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("agent-airlift/0.1")
        .build()?;

    let resp = client
        .get(tracker_url)
        .send()
        .context("Marginlab tracker request failed")?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "Marginlab tracker fetch failed ({}): {}",
            status,
            &text[..text.len().min(400)]
        );
    }
    Ok(text)
}

fn should_use_cache_for_unrecognized_marginlab(health: &Value) -> bool {
    health["status"] == "unknown"
        && health["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("no clear degradation status"))
}

/// Returns the default Marginlab tracker URL for a known provider.
pub fn default_tracker_url(provider: &str) -> Option<&'static str> {
    match provider {
        "claude-code" => Some("https://marginlab.ai/trackers/claude-code/"),
        "codex" => Some("https://marginlab.ai/trackers/codex/"),
        _ => None,
    }
}

fn normalize_marginlab_response(body: &str, provider: &str, source_url: &str) -> Value {
    let normalized = normalize_whitespace(body);
    let lower = normalized.to_lowercase();
    let status_window = marginlab_status_window(&lower);

    let (status, confidence, reason) = if status_window.contains("collecting baseline data")
        || status_window.contains("degradation detection paused")
    {
        (
            "unknown",
            0.0,
            "Marginlab tracker reports baseline collection is active and degradation detection is paused.",
        )
    } else if status_window.contains("degraded") {
        (
            "degraded",
            0.75,
            "Marginlab tracker reported Degraded for provider degradation status.",
        )
    } else if status_window.contains("nominal") {
        (
            "nominal",
            0.75,
            "Marginlab tracker reported Nominal for provider degradation status.",
        )
    } else {
        (
            "unknown",
            0.0,
            "Marginlab tracker was fetched, but no clear degradation status was found.",
        )
    };

    json!({
        "provider": provider,
        "status": status,
        "confidence": confidence,
        "reason": reason,
        "source": "marginlab",
        "source_url": source_url,
        "observed_at": chrono::Utc::now().to_rfc3339(),
    })
}

fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn marginlab_status_window(lower: &str) -> &str {
    let start = lower
        .find("degradation status")
        .or_else(|| lower.find("status"))
        .unwrap_or(0);
    let tail = &lower[start..];
    let end = tail
        .find("baseline pass rate")
        .or_else(|| tail.find("today's pass rate"))
        .unwrap_or(tail.len().min(1_200));
    &tail[..end]
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_provider_health_file_loading() {
        let path = Path::new("examples/provider-health/degraded.marginlab.cached.json");
        let health = load_provider_health("file", Some(path)).unwrap();
        assert_eq!(health["provider"].as_str(), Some("claude-code"));
        assert_eq!(health["status"].as_str(), Some("degraded"));
        assert!(health["reason"].as_str().unwrap().contains("Cached Marginlab fallback"));
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
    fn test_normalize_marginlab_nominal_tracker_text() {
        let body = "Codex gpt-5.5-xhigh Performance Tracker\n\
Last updated: May 30, 2026\n\
Status\n\
Degradation Status\n\
Shows if any time period has a statistically significant performance drop (p < 0.05).\n\
Nominal\n\
Baseline\n\
Baseline Pass Rate\n\
56 %";

        let h = normalize_marginlab_response(body, "codex", "https://marginlab.ai/trackers/codex/");

        assert_eq!(h["provider"], "codex");
        assert_eq!(h["status"], "nominal");
        assert_eq!(h["source"], "marginlab");
        assert_eq!(h["source_url"], "https://marginlab.ai/trackers/codex/");
        assert!(h["reason"].as_str().unwrap().contains("reported Nominal"));
    }

    #[test]
    fn test_normalize_marginlab_degraded_tracker_text() {
        let body = "Claude Code Performance Tracker\n\
Status\n\
Degradation Status\n\
Shows if any time period has a statistically significant performance drop (p < 0.05).\n\
Degraded\n\
Change Overview\n\
Regression\n\
30D Last Month";

        let h = normalize_marginlab_response(
            body,
            "claude-code",
            "https://marginlab.ai/trackers/claude-code/",
        );

        assert_eq!(h["provider"], "claude-code");
        assert_eq!(h["status"], "degraded");
        assert_eq!(h["source"], "marginlab");
        assert!(h["confidence"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn test_normalize_marginlab_collecting_baseline_is_unknown() {
        let body = "Claude Code Opus 4.8 Performance Tracker\n\
We are collecting a fresh Opus 4.8 baseline on SWE tasks before resuming statistical degradation detection.\n\
New model — collecting baseline data. Degradation detection paused.\n\
Status\n\
Collecting baseline data\n\
Baseline\n\
Collecting...";

        let h = normalize_marginlab_response(
            body,
            "claude-code",
            "https://marginlab.ai/trackers/claude-code/",
        );

        assert_eq!(h["status"], "unknown");
        assert_eq!(h["confidence"], 0.0);
        assert!(h["reason"].as_str().unwrap().contains("paused"));
    }

    #[test]
    fn test_marginlab_fetch_failure_uses_cache() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("cached.json");
        fs::write(
            &cache_path,
            serde_json::to_string_pretty(&json!({
                "provider": "claude-code",
                "status": "degraded",
                "confidence": 0.74,
                "reason": "Cached Marginlab fallback for test.",
                "source": "fixture",
                "source_url": "https://marginlab.ai/trackers/claude-code/",
                "observed_at": "2026-06-01T00:00:00Z"
            })).unwrap(),
        ).unwrap();

        let cfg = MarginlabConfig {
            provider: "claude-code",
            tracker_url: "not-a-valid-url",
        };
        let (health, raw, warnings) = load_provider_health_marginlab(&cfg, Some(&cache_path));

        assert!(raw.is_none());
        assert_eq!(health["source"].as_str(), Some("cached-marginlab"));
        assert_eq!(health["status"].as_str(), Some("degraded"));
        assert!(warnings.iter().any(|w| w.contains("Marginlab live fetch failed")));
    }

    #[test]
    fn test_marginlab_fetch_failure_without_cache_returns_unknown() {
        let cfg = MarginlabConfig {
            provider: "claude-code",
            tracker_url: "not-a-valid-url",
        };
        let (health, raw, warnings) = load_provider_health_marginlab(&cfg, None);

        assert!(raw.is_none());
        assert_eq!(health["status"].as_str(), Some("unknown"));
        assert_eq!(health["source"].as_str(), Some("marginlab"));
        assert!(warnings.iter().any(|w| w.contains("unknown")));
    }
}
