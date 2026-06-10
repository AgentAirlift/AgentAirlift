pub mod cli;
pub mod config;
pub mod session_import;
pub mod canonical;
pub mod repo_snapshot;
pub mod provider_health;
pub mod exporters;
pub mod native_session;
pub mod audit;
pub mod fs_util;

use clap::Parser;
use std::fs;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    
    match cli.command {
        cli::Commands::Migrate {
            session,
            project,
            out,
            source,
            targets,
            provider_health,
            provider_health_file,
            provider_health_cache_file,
            skip_native_install,
            native_home,
        } => run_migration(
            session, project, out, source, targets,
            provider_health, provider_health_file,
            provider_health_cache_file,
            skip_native_install, native_home,
        ),
        cli::Commands::Health {
            source,
            out,
            provider_health,
            provider_health_file,
            provider_health_cache_file,
        } => run_health(
            source, out, provider_health, provider_health_file, provider_health_cache_file,
        ),
    }
}

fn run_migration(
    session: String,
    project: String,
    out: String,
    source: String,
    targets: Vec<String>,
    provider_health: String,
    provider_health_file: Option<String>,
    provider_health_cache_file: Option<String>,
    skip_native_install: bool,
    native_home: Option<String>,
) -> anyhow::Result<()> {
    println!("🚀 Agent Airlift Demo");
    println!("Source: {}", source);
    println!("Targets: {}", targets.join(", "));
    
    // Load configuration
    let config = config::Config::from_cli(
        session, project, out, source, targets, provider_health.clone(), provider_health_file.clone(),
    )?;
    
    // Create output directories
    config.create_output_dirs()?;
    println!("📁 Output directory: {}", config.output_dir.display());
    
    // 1. Import session
    println!("📥 Importing session...");
    let (turns, mut diagnostics) = session_import::import_session(&config.session_path)?;
    println!("   Parsed {} turns ({} format, confidence {:.2})",
        diagnostics.turns_imported, diagnostics.detected_format, diagnostics.format_confidence);
    if !diagnostics.warnings.is_empty() {
        println!("   ⚠️  {} warnings during import", diagnostics.warnings.len());
    }
    
    // Save raw session
    let raw_session_path = config.output_dir.join("raw/source-session.jsonl");
    fs::copy(&config.session_path, raw_session_path)?;
    
    // 2. Create repo snapshot
    println!("📦 Creating repository snapshot...");
    let repo_snapshot = repo_snapshot::create_repo_snapshot(&config.project_path)?;
    fs_util::write_json_pretty(
        &config.output_dir.join("raw/repo-snapshot.json"),
        &repo_snapshot,
    )?;
    
    // 3. Load provider health
    println!("🏥 Loading provider health...");
    let (provider_health_data, health_warnings) = if provider_health == "marginlab" {
        let tracker_url = provider_health::default_tracker_url(&config.source_provider)
            .ok_or_else(|| anyhow::anyhow!(
                "No default Marginlab tracker URL for provider '{}'",
                config.source_provider
            ))?;
        let cfg = provider_health::MarginlabConfig {
            provider: &config.source_provider,
            tracker_url,
        };
        let cache_path = provider_health_cache_file.as_deref().map(std::path::Path::new);
        let (health, raw_marginlab, warnings) =
            provider_health::load_provider_health_marginlab(&cfg, cache_path);
        if let Some(ref raw) = raw_marginlab {
            fs::write(config.output_dir.join("raw/marginlab-response.html"), raw)?;
        }
        (health, warnings)
    } else {
        let file_path = provider_health_file.as_deref().map(std::path::Path::new);
        let health = provider_health::load_provider_health(&provider_health, file_path)?;
        (health, vec![])
    };
    fs_util::write_json_pretty(
        &config.output_dir.join("raw/provider-health.json"),
        &provider_health_data,
    )?;
    
    // 4. Normalize session
    println!("🔄 Normalizing session...");
    let (canonical_turns, canonical_warnings, dropped_fields) =
        canonical::normalize_turns(turns, &config.source_provider);
    fs_util::write_json_pretty(
        &config.output_dir.join("normalized/canonical-session.json"),
        &serde_json::to_value(&canonical_turns)?,
    )?;
    
    // 5. Create replay session
    println!("🎬 Creating replay session...");
    let replay_lines: Vec<String> = canonical_turns.iter().map(|turn| {
        replay_line(turn)
    }).collect();
    fs_util::write_jsonl(
        &config.output_dir.join("replay/agent-airlift.session.jsonl"),
        &replay_lines,
    )?;
    
    // 6. Export for each target
    println!("📤 Exporting for targets...");
    for target in &config.target_providers {
        println!("   → {}", target);
        exporters::export_for_target(target, &canonical_turns, &config.output_dir.join("exports"))?;
    }

    // 6b. Install native resume-compatible sessions (claude-code / codex)
    let home = native_home
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let exports_dir = config.output_dir.join("exports");
    for target in &config.target_providers {
        if let Some(res) = native_session::install_native(
            target, &canonical_turns, &config.project_path, &home, &exports_dir, skip_native_install,
        )? {
            println!("✅ Native {} session: {}", target, res.path.display());
            println!("   To resume:");
            if let Some(cd) = &res.cd_hint {
                println!("     cd {}", cd);
            }
            println!("     {}", res.resume_cmd);
        }
    }
    
    // 7. Create handoff documentation
    println!("📝 Creating handoff documentation...");
    let handoff_ctx = exporters::HandoffContext {
        source: &config.source_provider,
        targets: &config.target_providers,
        repo_snapshot: Some(&repo_snapshot),
        provider_health: Some(&provider_health_data),
    };
    exporters::create_handoff_docs(&canonical_turns, &handoff_ctx, &config.output_dir.join("exports"))?;
    
    // 8. Create audit reports
    println!("📊 Creating audit reports...");
    diagnostics.warnings.extend(health_warnings);
    diagnostics.warnings.extend(canonical_warnings);
    audit::create_audit_report(
        &canonical_turns,
        &diagnostics,
        &dropped_fields,
        &config.source_provider,
        &config.output_dir.join("audit"),
    )?;
    
    // Summary
    println!("\n✅ Demo completed successfully!");
    println!("📊 Summary:");
    println!("   - Turns processed: {}", canonical_turns.len());
    println!("   - Import warnings: {}", diagnostics.warnings.len());
    println!("   - Targets exported: {}", config.target_providers.join(", "));
    println!("   - Output location: {}", config.output_dir.display());

    Ok(())
}

fn replay_line(turn: &canonical::CanonicalTurn) -> String {
    serde_json::json!({
        "type": "airlift_replay_record",
        "replayable": matches!(turn.role.as_str(), "user" | "assistant"),
        "record_type": turn.record_type,
        "canonical_sha256": turn.canonical_sha256,
        "canonical": turn,
    })
    .to_string()
}

fn run_health(
    source: String,
    out: String,
    provider_health: String,
    provider_health_file: Option<String>,
    provider_health_cache_file: Option<String>,
) -> anyhow::Result<()> {
    let (health, warnings) = if provider_health == "marginlab" {
        let tracker_url = provider_health::default_tracker_url(&source)
            .ok_or_else(|| anyhow::anyhow!(
                "No default Marginlab tracker URL for provider '{}'",
                source
            ))?;
        let cfg = provider_health::MarginlabConfig {
            provider: &source,
            tracker_url,
        };
        let cache_path = provider_health_cache_file.as_deref().map(std::path::Path::new);
        let (health, _raw, warnings) =
            provider_health::load_provider_health_marginlab(&cfg, cache_path);
        (health, warnings)
    } else {
        let file_path = provider_health_file.as_deref().map(std::path::Path::new);
        (provider_health::load_provider_health(&provider_health, file_path)?, vec![])
    };

    let out_dir = std::path::PathBuf::from(&out);
    fs::create_dir_all(&out_dir)?;
    fs_util::write_json_pretty(&out_dir.join("provider-health.json"), &health)?;

    for w in &warnings {
        eprintln!("⚠️  {}", w);
    }
    println!("{}", serde_json::to_string_pretty(&health)?);
    println!(
        "AIRLIFT_HEALTH status={} provider={} confidence={} source={}",
        health["status"].as_str().unwrap_or("unknown"),
        health["provider"].as_str().unwrap_or(&source),
        health["confidence"].as_f64().unwrap_or(0.0),
        health["source"].as_str().unwrap_or("unknown"),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replay_line_wraps_non_message_records_as_metadata() {
        let turn = canonical::CanonicalTurn {
            id: "s1".into(),
            role: "summary".into(),
            content: "Compaction summary".into(),
            record_type: "summary".into(),
            canonical_sha256: "hash".into(),
            metadata: json!({}),
            ..Default::default()
        };

        let line: serde_json::Value = serde_json::from_str(&replay_line(&turn)).unwrap();
        assert_eq!(line["type"], "airlift_replay_record");
        assert_eq!(line["replayable"], false);
        assert_eq!(line["canonical"]["role"], "summary");
        assert_eq!(line["canonical_sha256"], "hash");
    }

    #[test]
    fn replay_line_marks_user_and_assistant_as_replayable() {
        let turn = canonical::CanonicalTurn {
            id: "u1".into(),
            role: "user".into(),
            content: "Continue".into(),
            record_type: "user".into(),
            metadata: json!({}),
            ..Default::default()
        };

        let line: serde_json::Value = serde_json::from_str(&replay_line(&turn)).unwrap();
        assert_eq!(line["replayable"], true);
    }
}
