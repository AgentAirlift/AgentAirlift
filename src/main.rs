pub mod cli;
pub mod config;
pub mod session_import;
pub mod canonical;
pub mod repo_snapshot;
pub mod provider_health;
pub mod exporters;
pub mod audit;
pub mod fs_util;
pub mod box_vault;

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
            apify_actor_id,
            apify_task_id,
            apify_input_url,
            apify_cache_file,
            box_upload,
            box_dry_run,
            box_parent_folder_id,
        } => run_migration(
            session, project, out, source, targets,
            provider_health, provider_health_file,
            apify_actor_id, apify_task_id, apify_input_url, apify_cache_file,
            box_upload, box_dry_run, box_parent_folder_id,
        ),
        cli::Commands::Health {
            source,
            out,
            provider_health,
            provider_health_file,
            apify_actor_id,
            apify_task_id,
            apify_input_url,
            apify_cache_file,
        } => run_health(
            source, out, provider_health, provider_health_file,
            apify_actor_id, apify_task_id, apify_input_url, apify_cache_file,
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
    apify_actor_id: Option<String>,
    apify_task_id: Option<String>,
    apify_input_url: Option<String>,
    apify_cache_file: Option<String>,
    box_upload: bool,
    box_dry_run: bool,
    box_parent_folder_id: Option<String>,
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
    let (provider_health_data, apify_warnings) = if provider_health == "apify" {
        // Only read token when apify mode is explicitly requested
        let token = std::env::var("APIFY_API_TOKEN").unwrap_or_default();
        let actor_id_env = std::env::var("APIFY_ACTOR_ID").ok();
        let task_id_env = std::env::var("APIFY_TASK_ID").ok();
        let cfg = provider_health::ApifyConfig {
            token: &token,
            actor_id: apify_actor_id.as_deref().or(actor_id_env.as_deref()),
            task_id: apify_task_id.as_deref().or(task_id_env.as_deref()),
            input_url: apify_input_url.as_deref()
                .or_else(|| provider_health::default_tracker_url(&config.source_provider)),
            provider: &config.source_provider,
        };
        let cache_path = apify_cache_file.as_deref().map(std::path::Path::new);
        let (health, raw_apify, warnings) = provider_health::load_provider_health_apify(&cfg, cache_path);
        // Save raw Apify response if we got one
        if let Some(ref raw) = raw_apify {
            fs_util::write_json_pretty(
                &config.output_dir.join("raw/apify-response.json"),
                raw,
            )?;
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
    let (canonical_turns, _, dropped_fields) = canonical::normalize_turns(turns, &config.source_provider);
    fs_util::write_json_pretty(
        &config.output_dir.join("normalized/canonical-session.json"),
        &serde_json::to_value(&canonical_turns)?,
    )?;
    
    // 5. Create replay session
    println!("🎬 Creating replay session...");
    let replay_lines: Vec<String> = canonical_turns.iter().map(|turn| {
        serde_json::json!({
            "id": turn.id,
            "role": turn.role,
            "content": turn.content,
            "timestamp": turn.timestamp,
        }).to_string()
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
    diagnostics.warnings.extend(apify_warnings);
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

    // ── Box vault ─────────────────────────────────────────────────────────────
    let project_name = config.project_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let root_folder_name = format!("AgentAirlift-{}-{}", project_name, timestamp);
    let audit_dir = config.output_dir.join("audit");

    if box_dry_run {
        box_vault::dry_run(&config.output_dir, &root_folder_name, &audit_dir)?;
    } else if box_upload {
        // Only read credentials when --box-upload is explicitly passed
        let token = std::env::var("BOX_DEVELOPER_TOKEN")
            .map_err(|_| anyhow::anyhow!("BOX_DEVELOPER_TOKEN env var is not set"))?;
        let parent_id = box_parent_folder_id
            .or_else(|| std::env::var("BOX_PARENT_FOLDER_ID").ok())
            .ok_or_else(|| anyhow::anyhow!(
                "Box parent folder ID is required: pass --box-parent-folder-id or set BOX_PARENT_FOLDER_ID"
            ))?;
        let cfg = box_vault::BoxConfig {
            token,
            parent_folder_id: parent_id,
            root_folder_name,
        };
        box_vault::upload(&cfg, &config.output_dir, &audit_dir)?;
    } else {
        println!("\nBox upload disabled. Local artifacts only.");
    }

    Ok(())
}

fn run_health(
    source: String,
    out: String,
    provider_health: String,
    provider_health_file: Option<String>,
    apify_actor_id: Option<String>,
    apify_task_id: Option<String>,
    apify_input_url: Option<String>,
    apify_cache_file: Option<String>,
) -> anyhow::Result<()> {
    let (health, warnings) = if provider_health == "apify" {
        let token = std::env::var("APIFY_API_TOKEN").unwrap_or_default();
        let actor_id_env = std::env::var("APIFY_ACTOR_ID").ok();
        let task_id_env = std::env::var("APIFY_TASK_ID").ok();
        let cfg = provider_health::ApifyConfig {
            token: &token,
            actor_id: apify_actor_id.as_deref().or(actor_id_env.as_deref()),
            task_id: apify_task_id.as_deref().or(task_id_env.as_deref()),
            input_url: apify_input_url.as_deref()
                .or_else(|| provider_health::default_tracker_url(&source)),
            provider: &source,
        };
        let cache_path = apify_cache_file.as_deref().map(std::path::Path::new);
        let (health, _raw, warnings) = provider_health::load_provider_health_apify(&cfg, cache_path);
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