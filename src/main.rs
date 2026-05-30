pub mod cli;
pub mod config;
pub mod session_import;
pub mod canonical;
pub mod repo_snapshot;
pub mod provider_health;
pub mod exporters;
pub mod audit;
pub mod fs_util;

use clap::Parser;
use std::fs;

fn main() -> anyhow::Result<()> {
    let cli = cli::Cli::parse();
    
    match cli.command {
        cli::Commands::Demo {
            session,
            project,
            out,
            source,
            targets,
            provider_health,
            provider_health_file,
        } => run_demo(session, project, out, source, targets, provider_health, provider_health_file),
    }
}

fn run_demo(
    session: String,
    project: String,
    out: String,
    source: String,
    targets: Vec<String>,
    provider_health: String,
    provider_health_file: Option<String>,
) -> anyhow::Result<()> {
    println!("🚀 Agent Airlift Demo");
    println!("Source: {}", source);
    println!("Targets: {}", targets.join(", "));
    
    // Load configuration
    let config = config::Config::from_cli(
        session, project, out, source, targets, provider_health, provider_health_file,
    )?;
    
    // Create output directories
    config.create_output_dirs()?;
    println!("📁 Output directory: {}", config.output_dir.display());
    
    // 1. Import session
    println!("📥 Importing session...");
    let (turns, import_warnings) = session_import::import_session(&config.session_path)?;
    println!("   Parsed {} turns", turns.len());
    if !import_warnings.is_empty() {
        println!("   ⚠️  {} warnings during import", import_warnings.len());
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
    let provider_health_data = provider_health::load_provider_health(
        &config.provider_health_source,
        config.provider_health_file.as_deref(),
    )?;
    fs_util::write_json_pretty(
        &config.output_dir.join("raw/provider-health.json"),
        &provider_health_data,
    )?;
    
    // 4. Normalize session
    println!("🔄 Normalizing session...");
    let (canonical_turns, _, dropped_fields) = canonical::normalize_turns(turns);
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
    audit::create_audit_report(
        &canonical_turns,
        &import_warnings,
        &dropped_fields,
        &config.output_dir.join("audit"),
    )?;
    
    // Summary
    println!("\n✅ Demo completed successfully!");
    println!("📊 Summary:");
    println!("   - Turns processed: {}", canonical_turns.len());
    println!("   - Import warnings: {}", import_warnings.len());
    println!("   - Targets exported: {}", config.target_providers.join(", "));
    println!("   - Output location: {}", config.output_dir.display());
    
    Ok(())
}