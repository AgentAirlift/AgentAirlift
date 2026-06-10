use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_SNAPSHOT_FILE_BYTES: u64 = 256 * 1024;

pub fn create_repo_snapshot(project_path: &Path) -> anyhow::Result<Value> {
    create_repo_snapshot_with_excludes(project_path, &[])
}

pub fn create_repo_snapshot_with_excludes(
    project_path: &Path,
    excluded_roots: &[PathBuf],
) -> anyhow::Result<Value> {
    let project_root = project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf());
    let excluded_roots = normalize_excluded_roots(&project_root, excluded_roots);
    let mut snapshot = json!({
        "project_path": project_root.to_string_lossy().to_string(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "files": [],
    });

    let mut files = Vec::new();

    for entry in WalkDir::new(&project_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path(), &project_root, &excluded_roots))
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Skip directories and symlinks; Path::is_file/read_to_string follow symlinks.
        if !entry.file_type().is_file() || entry.file_type().is_symlink() {
            continue;
        }

        if should_skip_file(path, &project_root, &excluded_roots) {
            continue;
        }

        let relative_path = path
            .strip_prefix(&project_root)
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

    files.sort_by(|a, b| {
        a["path"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["path"].as_str().unwrap_or_default())
    });

    if let Value::Object(ref mut obj) = snapshot {
        obj["files"] = Value::Array(files);
    }

    Ok(snapshot)
}

fn normalize_excluded_roots(project_root: &Path, excluded_roots: &[PathBuf]) -> Vec<PathBuf> {
    excluded_roots
        .iter()
        .map(|root| {
            let absolute = if root.is_absolute() {
                root.clone()
            } else {
                project_root.join(root)
            };
            absolute.canonicalize().unwrap_or(absolute)
        })
        .collect()
}

fn should_descend(path: &Path, project_root: &Path, excluded_roots: &[PathBuf]) -> bool {
    if path == project_root {
        return true;
    }
    !is_excluded_root(path, excluded_roots)
        && !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| is_pruned_dir_name(name))
}

fn should_skip_file(path: &Path, project_root: &Path, excluded_roots: &[PathBuf]) -> bool {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if file_name.ends_with(".log") || file_name == "Cargo.lock" {
        return true;
    }
    if is_sensitive_file_name(file_name) || is_excluded_root(path, excluded_roots) {
        return true;
    }
    if fs::metadata(path)
        .map(|m| m.len() > MAX_SNAPSHOT_FILE_BYTES)
        .unwrap_or(true)
    {
        return true;
    }
    path.strip_prefix(project_root)
        .ok()
        .is_some_and(|relative| {
            relative
                .components()
                .filter_map(|component| component.as_os_str().to_str())
                .any(|component| is_pruned_dir_name(component) || is_sensitive_file_name(component))
        })
}

fn is_excluded_root(path: &Path, excluded_roots: &[PathBuf]) -> bool {
    excluded_roots
        .iter()
        .any(|root| path == root || path.starts_with(root))
}

fn is_pruned_dir_name(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "target" | "node_modules" | "airlift-out" | "dist" | "build" | "__pycache__"
        )
}

fn is_sensitive_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with(".env")
        || matches!(
            lower.as_str(),
            "id_rsa"
                | "id_dsa"
                | "id_ecdsa"
                | "id_ed25519"
                | "credentials"
                | "credential"
                | "secrets"
                | "secret"
        )
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
        || lower.contains("credentials")
        || lower.contains("secret")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn file_paths(snapshot: &Value) -> Vec<String> {
        snapshot["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn snapshot_prunes_hidden_and_sensitive_directories() {
        let project = TempDir::new().unwrap();
        fs::create_dir_all(project.path().join(".git")).unwrap();
        fs::create_dir_all(project.path().join(".aws")).unwrap();
        fs::create_dir_all(project.path().join("src")).unwrap();
        fs::write(project.path().join(".git/config"), "remote = secret\n").unwrap();
        fs::write(
            project.path().join(".aws/credentials"),
            "aws_secret_access_key = abc\n",
        )
        .unwrap();
        fs::write(project.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let snapshot = create_repo_snapshot(project.path()).unwrap();
        let paths = file_paths(&snapshot);

        assert_eq!(paths, vec!["src/main.rs"]);
    }

    #[test]
    fn snapshot_prunes_output_directory_and_large_files() {
        let project = TempDir::new().unwrap();
        fs::create_dir_all(project.path().join("airlift-out/raw")).unwrap();
        fs::create_dir_all(project.path().join("src")).unwrap();
        fs::write(
            project.path().join("airlift-out/raw/source-session.jsonl"),
            "session bytes\n",
        )
        .unwrap();
        fs::write(project.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(project.path().join("src/large.txt"), "x".repeat(300 * 1024)).unwrap();

        let snapshot = create_repo_snapshot_with_excludes(
            project.path(),
            &[project.path().join("airlift-out")],
        )
        .unwrap();
        let paths = file_paths(&snapshot);

        assert_eq!(paths, vec!["src/main.rs"]);
    }

    #[test]
    fn snapshot_does_not_follow_symlinked_files() {
        let project = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::create_dir_all(project.path().join("src")).unwrap();
        fs::write(project.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(outside.path().join("secret.txt"), "private key bytes\n").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            project.path().join("src/notes.txt"),
        )
        .unwrap();

        let snapshot = create_repo_snapshot(project.path()).unwrap();
        let paths = file_paths(&snapshot);

        assert_eq!(paths, vec!["src/main.rs"]);
    }
}
