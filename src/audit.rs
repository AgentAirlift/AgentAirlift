use serde_json::{json, Value};
use std::fs;
use crate::canonical::CanonicalTurn;

pub fn create_audit_report(
    turns: &[CanonicalTurn],
    import_warnings: &[String],
    dropped_fields: &Value,
    output_dir: &std::path::Path,
) -> anyhow::Result<()> {
    // Create conversion report
    let report = format!("# Conversion Report\n\n## Summary\n- Total turns converted: {}\n- Import warnings: {}\n- Source provider: Migrated\n\n## Turn Statistics\n\n{}",
        turns.len(),
        import_warnings.len(),
        turns.iter().map(|turn| format!("- {}: {} ({} chars)", turn.id, turn.role, turn.content.len())).collect::<Vec<_>>().join("\n")
    );
    
    fs::write(output_dir.join("conversion-report.md"), report)?;
    
    // Create warnings JSON
    let warnings = json!({
        "count": import_warnings.len(),
        "warnings": import_warnings,
    });
    
    fs::write(
        output_dir.join("warnings.json"),
        serde_json::to_string_pretty(&warnings)?,
    )?;
    
    // Create dropped fields JSON
    fs::write(
        output_dir.join("dropped-fields.json"),
        serde_json::to_string_pretty(dropped_fields)?,
    )?;
    
    Ok(())
}