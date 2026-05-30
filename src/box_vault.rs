use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Box API response types (only fields we need) ──────────────────────────────

#[derive(Deserialize)]
struct BoxFolder {
    id: String,
    #[allow(dead_code)]
    name: String,
}

#[derive(Deserialize)]
struct BoxUploadResponse {
    entries: Vec<BoxFileEntry>,
}

#[derive(Deserialize)]
struct BoxFileEntry {
    id: String,
    #[allow(dead_code)]
    name: String,
}

// ── Manifest types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct UploadManifest {
    pub mode: String,
    pub box_root_folder_id: String,
    pub box_root_folder_url: String,
    pub uploaded_at: String,
    pub files: Vec<FileEntry>,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
pub struct FileEntry {
    pub local_path: String,
    pub box_folder_id: String,
    pub box_file_id: String,
    pub status: String,
    pub error: Option<String>,
}

// ── Public entry point ────────────────────────────────────────────────────────

pub struct BoxConfig {
    pub token: String,
    pub parent_folder_id: String,
    pub root_folder_name: String,
}

/// Collect all files under `output_dir` that should be uploaded.
/// Returns (relative_path_from_output_dir, absolute_path) pairs.
pub fn collect_upload_files(output_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(output_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let abs = entry.path().to_path_buf();
        let rel = abs
            .strip_prefix(output_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        files.push((rel, abs));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Dry-run: build manifest without calling Box.
pub fn dry_run(
    output_dir: &Path,
    root_folder_name: &str,
    audit_dir: &Path,
) -> anyhow::Result<()> {
    let files = collect_upload_files(output_dir);
    let entries: Vec<FileEntry> = files
        .iter()
        .map(|(rel, _)| FileEntry {
            local_path: rel.clone(),
            box_folder_id: String::new(),
            box_file_id: String::new(),
            status: "would-upload".into(),
            error: None,
        })
        .collect();

    let manifest = UploadManifest {
        mode: "dry-run".into(),
        box_root_folder_id: String::new(),
        box_root_folder_url: String::new(),
        uploaded_at: chrono::Utc::now().to_rfc3339(),
        files: entries,
        warnings: vec![],
    };

    println!("Box dry run:");
    println!("  Would create folder: {}", root_folder_name);
    println!("  Would upload {} files.", files.len());

    write_manifest(audit_dir, &manifest)
}

/// Live upload: create Box folder tree and upload all files.
pub fn upload(cfg: &BoxConfig, output_dir: &Path, audit_dir: &Path) -> anyhow::Result<()> {
    let client = reqwest::blocking::Client::new();

    // 1. Create root folder
    let root_id = create_folder(&client, &cfg.token, &cfg.root_folder_name, &cfg.parent_folder_id)
        .context("Failed to create Box root folder")?;
    let root_url = format!("https://app.box.com/folder/{}", root_id);
    println!("Box upload:");
    println!("  ✅ Created Box vault: {}", root_url);

    // 2. Collect files
    let files = collect_upload_files(output_dir);

    // 3. Ensure subfolder IDs (lazily created, cached)
    let mut folder_cache: HashMap<String, String> = HashMap::new();
    folder_cache.insert(String::new(), root_id.clone());

    let mut entries: Vec<FileEntry> = Vec::new();
    let mut uploaded = 0usize;
    let mut failed = 0usize;

    for (rel, abs) in &files {
        // Determine the Box folder for this file
        let parent_rel = match rel.rfind('/') {
            Some(i) => rel[..i].to_string(),
            None => String::new(),
        };
        let box_folder_id = match ensure_folder_path(
            &client,
            &cfg.token,
            &parent_rel,
            &mut folder_cache,
        ) {
            Ok(id) => id,
            Err(e) => {
                entries.push(FileEntry {
                    local_path: rel.clone(),
                    box_folder_id: String::new(),
                    box_file_id: String::new(),
                    status: "failed".into(),
                    error: Some(format!("folder creation failed: {}", e)),
                });
                failed += 1;
                continue;
            }
        };

        let filename = abs.file_name().unwrap().to_string_lossy().to_string();
        match upload_file(&client, &cfg.token, abs, &filename, &box_folder_id) {
            Ok(file_id) => {
                entries.push(FileEntry {
                    local_path: rel.clone(),
                    box_folder_id: box_folder_id.clone(),
                    box_file_id: file_id,
                    status: "uploaded".into(),
                    error: None,
                });
                uploaded += 1;
            }
            Err(e) => {
                entries.push(FileEntry {
                    local_path: rel.clone(),
                    box_folder_id: box_folder_id.clone(),
                    box_file_id: String::new(),
                    status: "failed".into(),
                    error: Some(e.to_string()),
                });
                failed += 1;
            }
        }
    }

    println!("  ✅ Uploaded {} files", uploaded);
    if failed > 0 {
        println!("  ⚠️  Failed {} files; see audit/upload-manifest.json", failed);
    }

    let mode = if failed > 0 { "failed" } else { "uploaded" };
    let manifest = UploadManifest {
        mode: mode.into(),
        box_root_folder_id: root_id,
        box_root_folder_url: root_url,
        uploaded_at: chrono::Utc::now().to_rfc3339(),
        files: entries,
        warnings: vec![],
    };
    write_manifest(audit_dir, &manifest)
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn create_folder(
    client: &reqwest::blocking::Client,
    token: &str,
    name: &str,
    parent_id: &str,
) -> anyhow::Result<String> {
    let body = json!({ "name": name, "parent": { "id": parent_id } });
    let resp = client
        .post("https://api.box.com/2.0/folders")
        .bearer_auth(token)
        .json(&body)
        .send()
        .context("POST /folders network error")?;

    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("Box create folder '{}' failed ({}): {}", name, status, text);
    }
    let folder: BoxFolder = serde_json::from_str(&text)
        .context("Failed to parse Box folder response")?;
    Ok(folder.id)
}

/// Ensure all path components exist in Box, creating them if needed.
/// `rel_path` is like "exports/.kiro/specs/agent-airlift-handoff"
fn ensure_folder_path(
    client: &reqwest::blocking::Client,
    token: &str,
    rel_path: &str,
    cache: &mut HashMap<String, String>,
) -> anyhow::Result<String> {
    if let Some(id) = cache.get(rel_path) {
        return Ok(id.clone());
    }
    // Build from root down
    let parts: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
    let mut current_rel = String::new();
    let mut parent_id = cache.get("").unwrap().clone();
    for part in parts {
        let next_rel = if current_rel.is_empty() {
            part.to_string()
        } else {
            format!("{}/{}", current_rel, part)
        };
        if let Some(id) = cache.get(&next_rel) {
            parent_id = id.clone();
        } else {
            let id = create_folder(client, token, part, &parent_id)?;
            cache.insert(next_rel.clone(), id.clone());
            parent_id = id;
        }
        current_rel = next_rel;
    }
    Ok(parent_id)
}

fn upload_file(
    client: &reqwest::blocking::Client,
    token: &str,
    path: &Path,
    name: &str,
    parent_id: &str,
) -> anyhow::Result<String> {
    let attrs = json!({ "name": name, "parent": { "id": parent_id } }).to_string();
    let file_bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let form = reqwest::blocking::multipart::Form::new()
        .text("attributes", attrs)
        .part(
            "file",
            reqwest::blocking::multipart::Part::bytes(file_bytes)
                .file_name(name.to_string())
                .mime_str("application/octet-stream")?,
        );

    let resp = client
        .post("https://upload.box.com/api/2.0/files/content")
        .bearer_auth(token)
        .multipart(form)
        .send()
        .context("POST /files/content network error")?;

    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        bail!("Box upload '{}' failed ({}): {}", name, status, text);
    }
    let upload: BoxUploadResponse = serde_json::from_str(&text)
        .context("Failed to parse Box upload response")?;
    upload
        .entries
        .into_iter()
        .next()
        .map(|e| e.id)
        .context("Box upload response had no entries")
}

fn write_manifest(audit_dir: &Path, manifest: &UploadManifest) -> anyhow::Result<()> {
    std::fs::create_dir_all(audit_dir)?;
    let path = audit_dir.join("upload-manifest.json");
    let json = serde_json::to_string_pretty(manifest)?;
    std::fs::write(path, json)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_output_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        for sub in &["raw", "normalized", "exports", "audit"] {
            std::fs::create_dir_all(base.join(sub)).unwrap();
        }
        std::fs::write(base.join("raw/source-session.jsonl"), b"{}").unwrap();
        std::fs::write(base.join("normalized/canonical-session.json"), b"[]").unwrap();
        std::fs::write(base.join("exports/HANDOFF.md"), b"# Handoff").unwrap();
        std::fs::write(base.join("exports/AGENTS.md"), b"# Agents").unwrap();
        // nested kiro spec
        std::fs::create_dir_all(base.join("exports/.kiro/specs/agent-airlift-handoff")).unwrap();
        std::fs::write(
            base.join("exports/.kiro/specs/agent-airlift-handoff/requirements.md"),
            b"# Req",
        ).unwrap();
        dir
    }

    #[test]
    fn test_collect_upload_files_finds_all_files() {
        let dir = make_output_dir();
        let files = collect_upload_files(dir.path());
        let paths: Vec<&str> = files.iter().map(|(r, _)| r.as_str()).collect();
        assert!(paths.contains(&"exports/HANDOFF.md"));
        assert!(paths.contains(&"exports/AGENTS.md"));
        assert!(paths.contains(&"raw/source-session.jsonl"));
        assert!(paths.iter().any(|p| p.contains("requirements.md")));
    }

    #[test]
    fn test_dry_run_writes_manifest() {
        let dir = make_output_dir();
        let audit_dir = dir.path().join("audit");
        dry_run(dir.path(), "AgentAirlift-test-20240101", &audit_dir).unwrap();

        let manifest_path = audit_dir.join("upload-manifest.json");
        assert!(manifest_path.exists());
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["mode"], "dry-run");
        assert!(v["files"].as_array().unwrap().len() > 0);
        let statuses: Vec<&str> = v["files"]
            .as_array().unwrap()
            .iter()
            .map(|f| f["status"].as_str().unwrap())
            .collect();
        assert!(statuses.iter().all(|s| *s == "would-upload"));
    }
}
