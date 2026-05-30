//! Writes resume-compatible native session files for `codex` and `claude-code`.
//!
//! Two storage models are honored:
//! - Codex: date-bucketed `~/.codex/sessions/Y/M/D/rollout-<ts>-<uuid>.jsonl`;
//!   the project dir is carried *inside* line 1 (`session_meta.cwd`).
//! - Claude Code: project-bucketed `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`;
//!   the project dir *is* the (`/`→`-` encoded) folder name.
//!
//! A native-format copy is always written into `exports/native/<target>/` (for
//! the Box audit bundle); unless `skip_install`, the same file is installed into
//! the real store so the target tool can resume it.

use crate::canonical::CanonicalTurn;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MIGRATION_TYPE: &str = "agent-airlift-migration";

pub struct NativeWriteResult {
    pub path: PathBuf,
    pub session_id: String,
    pub resume_cmd: String,
    pub cd_hint: Option<String>,
}

/// Writes the native-format session for `codex`/`claude-code`; `Ok(None)` for others.
pub fn install_native(
    target: &str,
    turns: &[CanonicalTurn],
    project_path: &Path,
    home: &Path,
    exports_dir: &Path,
    skip_install: bool,
) -> anyhow::Result<Option<NativeWriteResult>> {
    if turns.is_empty() {
        return Ok(None);
    }
    let source = turns[0].source.clone();
    match target {
        "codex" => install_codex(turns, project_path, home, exports_dir, skip_install, &source).map(Some),
        "claude-code" => install_claude(turns, project_path, home, exports_dir, skip_install, &source).map(Some),
        _ => Ok(None),
    }
}

// ── codex ──────────────────────────────────────────────────────────────────

fn install_codex(
    turns: &[CanonicalTurn],
    project_path: &Path,
    home: &Path,
    exports_dir: &Path,
    skip_install: bool,
    source: &str,
) -> anyhow::Result<NativeWriteResult> {
    let cwd = abs_project(project_path);
    let ts = session_time(turns);
    let digest = origin_digest(turns);
    let session_id = new_id();
    let lines = codex_lines(turns, &session_id, &cwd, &ts, source, &digest);

    let filename = format!("rollout-{}-{}.jsonl", ts.format("%Y-%m-%dT%H-%M-%S"), session_id);
    let export_copy = exports_dir.join("native/codex").join(&filename);
    write_lines(&export_copy, &lines)?;

    let mut written = export_copy;
    if !skip_install {
        let base = home.join(".codex/sessions");
        written = match find_existing(&base, true, source, &digest) {
            Some(existing) => existing,
            None => {
                let dir = base
                    .join(ts.format("%Y").to_string())
                    .join(ts.format("%m").to_string())
                    .join(ts.format("%d").to_string());
                let path = dir.join(&filename);
                write_lines(&path, &lines)?;
                path
            }
        };
    }

    let sid = codex_id_from_path(&written).unwrap_or(session_id);
    Ok(NativeWriteResult {
        resume_cmd: format!("codex resume {}", sid),
        session_id: sid,
        path: written,
        cd_hint: None,
    })
}

fn codex_lines(
    turns: &[CanonicalTurn],
    session_id: &str,
    cwd: &str,
    ts: &DateTime<Utc>,
    source: &str,
    digest: &str,
) -> Vec<String> {
    let iso = ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut lines = vec![json!({
        "timestamp": iso,
        "type": "session_meta",
        "payload": {
            "id": session_id, "timestamp": iso, "cwd": cwd,
            "originator": "agent-airlift", "cli_version": "agent-airlift",
            "source": "cli", "model_provider": "openai"
        }
    })
    .to_string()];

    for t in turns {
        let tts = entry_ts(t, &iso);
        match t.role.as_str() {
            "user" => lines.push(
                json!({"timestamp": tts, "type": "event_msg",
                       "payload": {"type": "user_message", "message": t.content}})
                .to_string(),
            ),
            "assistant" => {
                lines.push(
                    json!({"timestamp": tts, "type": "event_msg",
                           "payload": {"type": "agent_message", "message": t.content, "phase": "final_answer"}})
                    .to_string(),
                );
                lines.push(
                    json!({"timestamp": tts, "type": "response_item",
                           "payload": {"type": "message", "role": "assistant",
                                       "content": [{"type": "output_text", "text": t.content}],
                                       "phase": "final_answer"}})
                    .to_string(),
                );
            }
            _ => {}
        }
    }
    lines.push(migration_meta(source, turns.len(), digest).to_string());
    lines
}

/// Codex rollout stem is `rollout-<date>-<uuid>`; the uuid is the last 5 `-` parts.
fn codex_id_from_path(p: &Path) -> Option<String> {
    let stem = p.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.split('-').collect();
    (parts.len() >= 5).then(|| parts[parts.len() - 5..].join("-"))
}

// ── claude code ──────────────────────────────────────────────────────────────

fn install_claude(
    turns: &[CanonicalTurn],
    project_path: &Path,
    home: &Path,
    exports_dir: &Path,
    skip_install: bool,
    source: &str,
) -> anyhow::Result<NativeWriteResult> {
    let cwd = abs_project(project_path);
    let encoded = cwd.replace('/', "-");
    let ts = session_time(turns);
    let digest = origin_digest(turns);
    let session_id = new_id();
    let lines = claude_lines(turns, &session_id, &ts, source, &digest);

    let filename = format!("{}.jsonl", session_id);
    let export_copy = exports_dir.join("native/claude-code").join(&filename);
    write_lines(&export_copy, &lines)?;

    let mut written = export_copy;
    if !skip_install {
        let proj_dir = home.join(".claude/projects").join(&encoded);
        written = match find_existing(&proj_dir, false, source, &digest) {
            Some(existing) => existing,
            None => {
                let path = proj_dir.join(&filename);
                write_lines(&path, &lines)?;
                path
            }
        };
    }

    let sid = written
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .unwrap_or(session_id);
    Ok(NativeWriteResult {
        resume_cmd: format!("claude --resume {}", sid),
        session_id: sid,
        path: written,
        cd_hint: Some(cwd),
    })
}

fn claude_lines(
    turns: &[CanonicalTurn],
    session_id: &str,
    ts: &DateTime<Utc>,
    source: &str,
    digest: &str,
) -> Vec<String> {
    let iso = ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut lines = vec![json!({
        "type": "progress", "sessionId": session_id, "timestamp": iso,
        "uuid": new_id(), "data": migration_meta(source, turns.len(), digest)
    })
    .to_string()];

    let mut parent: Option<String> = None;
    for t in turns {
        let content = match t.role.as_str() {
            "user" => json!(t.content),
            "assistant" => json!([{"type": "text", "text": t.content}]),
            _ => continue,
        };
        let uuid = new_id();
        lines.push(
            json!({
                "type": t.role, "sessionId": session_id, "timestamp": entry_ts(t, &iso),
                "uuid": uuid, "parentUuid": parent,
                "message": {"role": t.role, "content": content}
            })
            .to_string(),
        );
        parent = Some(uuid);
    }
    lines
}

// ── shared helpers ───────────────────────────────────────────────────────────

fn new_id() -> String {
    Uuid::new_v4().to_string().to_lowercase()
}

fn abs_project(project_path: &Path) -> String {
    std::fs::canonicalize(project_path)
        .unwrap_or_else(|_| project_path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn session_time(turns: &[CanonicalTurn]) -> DateTime<Utc> {
    turns
        .iter()
        .find_map(|t| DateTime::parse_from_rfc3339(&t.timestamp).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
}

fn entry_ts(turn: &CanonicalTurn, fallback: &str) -> String {
    if turn.timestamp.is_empty() {
        fallback.to_string()
    } else {
        turn.timestamp.clone()
    }
}

/// SHA-256 over the canonical history; stable across runs for the same session.
fn origin_digest(turns: &[CanonicalTurn]) -> String {
    let mut hasher = Sha256::new();
    for t in turns {
        hasher.update(t.role.as_bytes());
        hasher.update([0x1f]);
        hasher.update(t.timestamp.as_bytes());
        hasher.update([0x1f]);
        hasher.update(t.content.as_bytes());
        hasher.update([0x1e]);
    }
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

fn migration_meta(source: &str, count: usize, digest: &str) -> Value {
    json!({
        "type": MIGRATION_TYPE,
        "originId": digest,
        "originSource": source,
        "originMessageCount": count,
        "originDigest": digest,
    })
}

fn write_lines(path: &Path, lines: &[String]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", lines.join("\n")))?;
    Ok(())
}

/// Finds an existing migrated session in `dir` with the same source + digest.
fn find_existing(dir: &Path, recursive: bool, source: &str, digest: &str) -> Option<PathBuf> {
    let files: Vec<PathBuf> = if recursive {
        walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| e.into_path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect()
    } else {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect()
    };
    files
        .into_iter()
        .find(|f| read_meta(f).is_some_and(|(s, d)| s == source && d == digest))
}

/// Reads `(originSource, originDigest)` from a session's migration meta line,
/// either bare (Codex trailing line) or wrapped in a `progress` line (Claude).
fn read_meta(file: &Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(file).ok()?;
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let meta = match v.get("type").and_then(Value::as_str) {
            Some(MIGRATION_TYPE) => Some(&v),
            Some("progress") => v.get("data"),
            _ => None,
        };
        if let Some(m) = meta {
            if m.get("type").and_then(Value::as_str) == Some(MIGRATION_TYPE) {
                if let (Some(s), Some(d)) = (
                    m.get("originSource").and_then(Value::as_str),
                    m.get("originDigest").and_then(Value::as_str),
                ) {
                    return Some((s.to_string(), d.to_string()));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn turns() -> Vec<CanonicalTurn> {
        vec![
            CanonicalTurn {
                id: "u1".into(), role: "user".into(), content: "Add a slug generator".into(),
                timestamp: "2026-05-29T19:00:05Z".into(), source: "claude-code".into(),
                ..Default::default()
            },
            CanonicalTurn {
                id: "a1".into(), role: "assistant".into(), content: "Done. Decision: base62.".into(),
                timestamp: "2026-05-29T19:00:18Z".into(), source: "claude-code".into(),
                ..Default::default()
            },
        ]
    }

    fn read(p: &Path) -> Vec<Value> {
        std::fs::read_to_string(p).unwrap().lines()
            .map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn codex_native_is_resume_shaped() {
        let home = TempDir::new().unwrap();
        let exports = TempDir::new().unwrap();
        let res = install_native("codex", &turns(), home.path(), home.path(), exports.path(), false)
            .unwrap().unwrap();

        // date-bucketed rollout file under ~/.codex/sessions, no colon in name
        let p = res.path.to_string_lossy();
        assert!(p.contains("/.codex/sessions/2026/05/29/"), "path: {}", p);
        let name = res.path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("rollout-2026-05-29T19-00-") && !name.contains(':'));
        assert_eq!(res.resume_cmd, format!("codex resume {}", res.session_id));

        let lines = read(&res.path);
        // line 1 is session_meta carrying cwd
        assert_eq!(lines[0]["type"], "session_meta");
        assert_eq!(lines[0]["payload"]["cwd"], home.path().canonicalize().unwrap().to_string_lossy().as_ref());
        // assistant turn => agent_message + response_item
        assert!(lines.iter().any(|l| l["payload"]["type"] == "agent_message"));
        assert!(lines.iter().any(|l| l["type"] == "response_item"));
        // trailing migration meta
        assert_eq!(lines.last().unwrap()["type"], "agent-airlift-migration");
        // export copy also written for Box
        assert!(exports.path().join("native/codex").read_dir().unwrap().count() == 1);
    }

    #[test]
    fn claude_native_chains_parent_uuid() {
        let home = TempDir::new().unwrap();
        let exports = TempDir::new().unwrap();
        let res = install_native("claude-code", &turns(), home.path(), home.path(), exports.path(), false)
            .unwrap().unwrap();

        let encoded = home.path().canonicalize().unwrap().to_string_lossy().replace('/', "-");
        assert!(res.path.to_string_lossy().contains(&format!("/.claude/projects/{}/", encoded)));
        assert_eq!(res.cd_hint.as_deref(), Some(home.path().canonicalize().unwrap().to_string_lossy().as_ref()));
        assert!(res.resume_cmd.starts_with("claude --resume "));

        let lines = read(&res.path);
        assert_eq!(lines[0]["type"], "progress");
        assert_eq!(lines[0]["data"]["type"], "agent-airlift-migration");
        // parentUuid chain: first message null, second equals first's uuid
        assert!(lines[1]["parentUuid"].is_null());
        assert_eq!(lines[2]["parentUuid"], lines[1]["uuid"]);
        assert_eq!(lines[1]["type"], "user");
        assert_eq!(lines[2]["message"]["content"][0]["type"], "text");
    }

    #[test]
    fn dedup_does_not_write_twice() {
        let home = TempDir::new().unwrap();
        let exports = TempDir::new().unwrap();
        let a = install_native("codex", &turns(), home.path(), home.path(), exports.path(), false)
            .unwrap().unwrap();
        let b = install_native("codex", &turns(), home.path(), home.path(), exports.path(), false)
            .unwrap().unwrap();
        assert_eq!(a.path, b.path, "second run must reuse the existing session");
        let count = walkdir::WalkDir::new(home.path().join(".codex/sessions"))
            .into_iter().filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl")).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn skip_install_writes_export_only() {
        let home = TempDir::new().unwrap();
        let exports = TempDir::new().unwrap();
        let res = install_native("codex", &turns(), home.path(), home.path(), exports.path(), true)
            .unwrap().unwrap();
        assert!(res.path.starts_with(exports.path()));
        assert!(!home.path().join(".codex").exists(), "must not touch real store when skipping");
    }

    #[test]
    fn other_targets_are_ignored() {
        let home = TempDir::new().unwrap();
        let exports = TempDir::new().unwrap();
        assert!(install_native("kiro", &turns(), home.path(), home.path(), exports.path(), false)
            .unwrap().is_none());
    }
}
