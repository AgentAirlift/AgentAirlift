use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

pub fn create_repo_snapshot(project_path: &Path) -> anyhow::Result<Value> {
    let mut snapshot = json!({
        "project_path": project_path.to_string_lossy().to_string(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "files": [],
    });
    
    let mut files = Vec::new();
    
    for entry in WalkDir::new(project_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        
        // Skip directories
        if !path.is_file() {
            continue;
        }
        
        // Skip common build artifacts and hidden files
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        if file_name.starts_with('.') || 
           file_name.ends_with(".log") || 
           file_name == "Cargo.lock" ||
           path.to_string_lossy().contains("target/") ||
           path.to_string_lossy().contains("node_modules/") {
            continue;
        }
        
        let relative_path = path.strip_prefix(project_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        
        match fs::read_to_string(path) {
            Ok(content) => {
                files.push(json!({
                    "path": relative_path,
                    "size": content.len(),
                    "lines": content.lines().count(),
                    "content": content,
                }));
            }
            Err(_) => {
                // Skip binary files or unreadable files
                continue;
            }
        }
    }
    
    if let Value::Object(ref mut obj) = snapshot {
        obj["files"] = Value::Array(files);
    }
    
    Ok(snapshot)
}