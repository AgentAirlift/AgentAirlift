use serde_json::{json, Value};
use crate::canonical::CanonicalTurn;

/// Lightweight context extracted from session + repo + health for doc generation.
pub struct HandoffContext<'a> {
    pub source: &'a str,
    pub targets: &'a [String],
    pub repo_snapshot: Option<&'a Value>,
    pub provider_health: Option<&'a Value>,
}

pub fn export_for_target(target: &str, turns: &[CanonicalTurn], output_dir: &std::path::Path) -> anyhow::Result<()> {
    match target {
        "codex" => export_codex(turns, output_dir),
        "claude-code" => export_claude_code(turns, output_dir),
        "kiro" => export_kiro(turns, output_dir),
        "opencode" => export_opencode(turns, output_dir),
        _ => Err(anyhow::anyhow!("Unsupported target: {}", target)),
    }
}

fn export_claude_code(turns: &[CanonicalTurn], output_dir: &std::path::Path) -> anyhow::Result<()> {
    // Readable export. The resume-compatible native session is written separately
    // by `native_session` into ~/.claude/projects and exports/native/claude-code/.
    let mut lines = vec![json!({
        "_meta": "agent-airlift-export",
        "format": "claude-code-like",
        "resume_compatible": false,
        "note": "Readable export. A native resume-compatible session is installed under ~/.claude/projects and exports/native/claude-code/."
    }).to_string()];
    for turn in turns {
        let line = json!({
            "type": turn.role,
            "uuid": turn.id,
            "timestamp": turn.timestamp,
            "message": {"role": turn.role, "content": turn.content},
            "tool_calls": turn.tool_calls,
            "tool_results": turn.tool_results,
            "agent_airlift_canonical": turn,
        });
        lines.push(line.to_string());
    }

    crate::fs_util::write_jsonl(&output_dir.join("claude-code-like.session.jsonl"), &lines)?;
    Ok(())
}

fn export_codex(turns: &[CanonicalTurn], output_dir: &std::path::Path) -> anyhow::Result<()> {
    // Leading meta line documents provenance. This is the readable export kept
    // alongside the resume-compatible native rollout written separately by
    // `native_session` into ~/.codex/sessions and exports/native/codex/.
    let mut lines = vec![json!({
        "_meta": "agent-airlift-export",
        "format": "codex-like",
        "resume_compatible": false,
        "note": "Readable export. A native resume-compatible rollout is installed under ~/.codex/sessions and exports/native/codex/."
    }).to_string()];
    for turn in turns {
        let line = json!({
            "id": turn.id,
            "role": turn.role,
            "content": turn.content,
            "timestamp": turn.timestamp,
            "tool_calls": turn.tool_calls,
            "source": turn.source,
            "tool_results": turn.tool_results,
            "agent_airlift_canonical": turn,
        });
        lines.push(line.to_string());
    }
    
    crate::fs_util::write_jsonl(&output_dir.join("codex-like.session.jsonl"), &lines)?;
    Ok(())
}

fn export_kiro(turns: &[CanonicalTurn], output_dir: &std::path::Path) -> anyhow::Result<()> {
    let kiro_session = json!({
        "version": "1.0",
        "resume_compatible": false,
        "note": "Readable Agent Airlift export for resuming context, not a native Kiro session file.",
        "turns": turns.iter().map(|turn| {
            json!({
                "id": turn.id,
                "role": turn.role,
                "content": turn.content,
                "timestamp": turn.timestamp,
                "tool_calls": turn.tool_calls,
                "metadata": turn.metadata,
                "tool_results": turn.tool_results,
                "source": turn.source,
                "agent_airlift_canonical": turn,
            })
        }).collect::<Vec<_>>(),
    });
    
    crate::fs_util::write_json_pretty(
        &output_dir.join("kiro-session.json"),
        &kiro_session,
    )?;
    
    // Create Kiro spec files
    let spec_dir = output_dir.join(".kiro/specs/agent-airlift-handoff");
    std::fs::create_dir_all(&spec_dir)?;
    let records = extract_handoff_records(turns);
    std::fs::write(
        spec_dir.join("requirements.md"),
        format!(
            "# Requirements\n\n## Current Objective\n> {}\n\n## Source Turns\n- {} turns migrated.\n",
            current_objective(turns),
            turns.len()
        ),
    )?;
    std::fs::write(
        spec_dir.join("design.md"),
        format!(
            "# Design\n\n## Preserved Decisions\n{}\n",
            records_section(&records)
        ),
    )?;
    std::fs::write(
        spec_dir.join("tasks.md"),
        format!(
            "# Tasks\n\n1. Review migrated session artifacts.\n2. Validate current objective: {}.\n3. Run project validation commands.\n",
            current_objective(turns)
        ),
    )?;
    
    Ok(())
}

fn export_opencode(turns: &[CanonicalTurn], output_dir: &std::path::Path) -> anyhow::Result<()> {
    let mut lines = vec![json!({
        "_meta": "agent-airlift-export",
        "format": "opencode-like",
        "resume_compatible": false,
        "note": "Readable export for resuming context, not a native OpenCode session file."
    }).to_string()];
    for turn in turns {
        let line = json!({
            "message_id": turn.id,
            "sender": turn.role,
            "text": turn.content,
            "created_at": turn.timestamp,
            "tool_calls": turn.tool_calls,
            "tool_results": turn.tool_results,
            "agent_airlift_canonical": turn,
        });
        lines.push(line.to_string());
    }
    
    crate::fs_util::write_jsonl(&output_dir.join("opencode-like.session.jsonl"), &lines)?;
    Ok(())
}

// ── extraction helpers ────────────────────────────────────────────────────────

fn first_user_content(turns: &[CanonicalTurn]) -> &str {
    turns.iter()
        .find(|t| t.role == "user")
        .map(|t| t.content.as_str())
        .unwrap_or("No user turns found in session.")
}

fn current_objective(turns: &[CanonicalTurn]) -> String {
    let last_user = turns.iter().filter(|t| t.role == "user").last();
    let last_turn = turns.last();
    match (last_user, last_turn) {
        (Some(u), Some(last)) if last.role == "assistant" => {
            format!("Validate completed request: {}", short_text(&u.content, 240))
        }
        (Some(u), _) => short_text(&u.content, 240),
        (None, _) => "Review migrated session and determine next action.".into(),
    }
}

fn short_text(text: &str, limit: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= limit {
        collapsed
    } else {
        format!("{}...", &collapsed[..limit])
    }
}

fn repo_file_list(snapshot: Option<&Value>) -> Vec<String> {
    snapshot
        .and_then(|s| s.get("files"))
        .and_then(|f| f.as_array())
        .map(|arr| arr.iter()
            .filter_map(|f| f.get("path").and_then(|p| p.as_str()))
            .map(|p| format!("- `{}`", p))
            .collect())
        .unwrap_or_default()
}

fn health_summary(health: Option<&Value>, evaluated_provider: &str) -> String {
    match health {
        None => "No provider health data available. Assume all providers operational.".into(),
        Some(h) => {
            // `source` = signal origin (marginlab, cached-marginlab, file, mock)
            // `provider` = evaluated provider (claude-code, etc.)
            let signal_source = h.get("source").and_then(|v| v.as_str()).unwrap_or("unknown");
            let provider      = h.get("provider").and_then(|v| v.as_str()).unwrap_or(evaluated_provider);
            let status        = h.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
            let reason        = h.get("reason").or_else(|| h.get("message"))
                                  .and_then(|v| v.as_str()).unwrap_or("");
            format!(
                "Provider health signal from `{}`: `{}` is **{}**. {}",
                signal_source, provider, status, reason
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HandoffRecord {
    label: &'static str,
    text: String,
    verbatim: bool,
    turn_id: String,
}

fn extract_handoff_records(turns: &[CanonicalTurn]) -> Vec<HandoffRecord> {
    let mut out = Vec::new();
    for t in turns {
        for line in t.content.lines() {
            let upper = line.to_ascii_uppercase();
            let trimmed = line.trim();
            let trimmed_upper = trimmed.to_ascii_uppercase();
            let label = if starts_with_record_label(&trimmed_upper, "DECISION") {
                Some("DECISION")
            } else if starts_with_record_label(&trimmed_upper, "RATIONALE") {
                Some("RATIONALE")
            } else if starts_with_record_label(&trimmed_upper, "STATUS") {
                Some("STATUS")
            } else {
                None
            };
            if let Some(label) = label {
                let record = HandoffRecord {
                    label,
                    text: line.to_string(),
                    verbatim: true,
                    turn_id: t.id.clone(),
                };
                if !out.contains(&record) {
                    out.push(record);
                }
            } else if let Some(idx) = upper.find("DECISION:") {
                let record = HandoffRecord {
                    label: "DECISION",
                    text: line[idx..].trim().to_string(),
                    verbatim: false,
                    turn_id: t.id.clone(),
                };
                if !out.contains(&record) {
                    out.push(record);
                }
            }
        }
    }
    out
}

fn starts_with_record_label(line_upper: &str, label: &str) -> bool {
    line_upper
        .strip_prefix(label)
        .is_some_and(|rest| rest.starts_with(':') || rest.starts_with('('))
}

fn records_section(records: &[HandoffRecord]) -> String {
    if records.is_empty() {
        return "- No explicit DECISION/RATIONALE/STATUS records detected. Review assistant turns for implicit choices.".into();
    }
    records
        .iter()
        .map(|r| {
            if r.verbatim {
                r.text.clone()
            } else {
                format!("- [{}] {}", r.turn_id, r.text)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Commands from assistant content heuristics AND structured Bash tool calls.
fn extract_commands(turns: &[CanonicalTurn]) -> Vec<String> {
    let mut out = Vec::new();
    let push = |cmd: &str, out: &mut Vec<String>| {
        let entry = format!("- `{}`", cmd.trim());
        if !cmd.trim().is_empty() && !out.contains(&entry) { out.push(entry); }
    };
    for t in turns {
        // Structured: Bash/shell tool calls expose the exact command.
        if let Some(calls) = t.tool_calls.as_array() {
            for c in calls {
                let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if name.eq_ignore_ascii_case("bash") || name.eq_ignore_ascii_case("shell") {
                    if let Some(cmd) = c.get("input").and_then(|i| i.get("command")).and_then(|v| v.as_str()) {
                        push(cmd, &mut out);
                    }
                }
            }
        }
        // Heuristic: command-looking lines in text content.
        for line in t.content.lines() {
            let l = line.trim();
            if l.starts_with("$ ") || l.starts_with("cargo ") || l.starts_with("npm ") || l.starts_with("git ") {
                push(l.trim_start_matches("$ "), &mut out);
            }
        }
    }
    if out.is_empty() {
        out.push("- No shell commands detected in session.".into());
    }
    out
}

fn extract_errors(turns: &[CanonicalTurn]) -> Vec<String> {
    let mut out = Vec::new();
    for t in turns {
        for line in t.content.lines() {
            let l = line.trim().to_lowercase();
            if l.contains("error") || l.contains("failed") || l.contains("panic") {
                let snippet = line.trim();
                let entry = format!("- {}", &snippet[..snippet.len().min(120)]);
                if !out.contains(&entry) { out.push(entry); }
            }
        }
    }
    if out.is_empty() {
        out.push("- No errors or failures detected in session content.".into());
    }
    out
}

fn work_completed(turns: &[CanonicalTurn]) -> Vec<String> {
    turns.iter()
        .filter(|t| t.role == "assistant")
        .enumerate()
        .map(|(i, t)| {
            let preview = t.content.lines().next().unwrap_or("(empty)").trim();
            format!("{}. [{}] {}", i + 1, t.id, &preview[..preview.len().min(100)])
        })
        .collect()
}

/// If the last turn is an assistant reply, the last user request was likely addressed.
/// Suggest validation rather than blindly repeating it.
fn next_task(turns: &[CanonicalTurn]) -> String {
    let last_user = turns.iter().filter(|t| t.role == "user").last();
    let last_turn = turns.last();

    match (last_user, last_turn) {
        (Some(u), Some(last)) if last.role == "assistant" => {
            // Last user request was followed by an assistant reply — likely completed.
            let preview = &u.content[..u.content.len().min(80)];
            format!(
                "Validate the completed task (\"{}\"), then continue with the next user request.",
                preview
            )
        }
        (Some(u), _) => {
            // Last turn is a user message — still open.
            u.content[..u.content.len().min(200)].to_string()
        }
        (None, _) => "No user turns found. Review session and determine next action.".into(),
    }
}

// ── public doc generators ─────────────────────────────────────────────────────

pub fn create_handoff_docs(
    turns: &[CanonicalTurn],
    ctx: &HandoffContext<'_>,
    output_dir: &std::path::Path,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;

    let objective   = first_user_content(turns);
    let files       = repo_file_list(ctx.repo_snapshot);
    let health      = health_summary(ctx.provider_health, ctx.source);
    let handoff_records = extract_handoff_records(turns);
    let commands    = extract_commands(turns);
    let errors      = extract_errors(turns);
    let targets_str = ctx.targets.join(", ");
    let objective_summary = current_objective(turns);
    let last_user   = turns.iter().filter(|t| t.role == "user").last()
                          .map(|t| t.content.as_str()).unwrap_or(objective);

    let files_section = if files.is_empty() {
        "No repo snapshot available. Inspect project root manually.".into()
    } else {
        files.join("\n")
    };

    let handoff = format!(
r#"# Agent Airlift Handoff

## Why This Handoff Exists
Session migrated from **{source}** to [{targets}] via Agent Airlift.
Provider health signal triggered failover or explicit export was requested.

## Source & Targets
- **Source agent:** `{source}`
- **Target agents:** {targets}

## Provider Health / Failover Signal
{health}

## Current Objective
> {objective}

## Current State Summary
- Total turns in session: {turn_count}
- Last user message: "{last_user}"

## Important Files
{files_section}

## Decisions Made
{records}

## Commands Run
{commands}

## Errors and Risks
{errors}

## Open Questions
- Has the current objective been fully achieved?
- Are there uncommitted changes in the working directory?
- Does the target agent need additional context not present in this session?

## Recommended Next Actions
1. Read `AGENTS.md` for project context and constraints.
2. Review the canonical session in `normalized/canonical-session.json`.
3. Inspect the repo snapshot in `raw/repo-snapshot.json` for file contents.
4. Continue from the last user message above.

## Resume Prompt
```
You are continuing a session migrated from {source}.
Objective: {objective}
Read AGENTS.md for full context, then continue where the session left off.
```

## Resume Compatibility
Target exports under `exports/` are readable, deterministic representations of the
session — not native session files. Native `resume` compatibility is not guaranteed.
Use them to seed a fresh session in the target tool, alongside this handoff.

## Evidence / Source Artifacts
- `raw/source-session.jsonl` — original session
- `raw/repo-snapshot.json` — project file snapshot
- `raw/provider-health.json` — provider health at export time
- `normalized/canonical-session.json` — normalized turns
- `replay/agent-airlift.session.jsonl` — replay-ready session
- `audit/conversion-report.md` — import diagnostics + turn stats
"#,
        source     = ctx.source,
        targets    = targets_str,
        health     = health,
        objective  = objective_summary,
        turn_count = turns.len(),
        last_user  = &last_user[..last_user.len().min(200)],
        files_section = files_section,
        records    = records_section(&handoff_records),
        commands   = commands.join("\n"),
        errors     = errors.join("\n"),
    );

    std::fs::write(output_dir.join("HANDOFF.md"), handoff)?;

    // ── AGENTS.md ─────────────────────────────────────────────────────────────
    let completed = work_completed(turns);
    let completed_str = if completed.is_empty() {
        "No assistant turns recorded.".into()
    } else {
        completed.join("\n")
    };

    let files_inspect = if files.is_empty() {
        "- Inspect project root — no snapshot available.".into()
    } else {
        files.iter().take(10).cloned().collect::<Vec<_>>().join("\n")
    };

    let next_task_str = next_task(turns);

    let agents = format!(
r#"# AGENTS.md — Agent Handoff Instructions

## Project Context
This project was being worked on in a `{source}` session with {turn_count} turns.
The session has been migrated to: {targets}.

## Current Objective
{objective}

## Constraints
- Do not repeat work already listed in "Work Already Completed".
- Do not call live provider-health endpoints unless explicitly requested; existing artifacts already include the provider-health signal.
- Keep changes deterministic and local unless explicitly instructed otherwise.
- Preserve unknown fields in any JSONL you read or write.

## Files to Inspect First
{files_inspect}

## Work Already Completed
{completed_str}

## Preserved Decision Records
{records}

## Do Not Repeat
- Re-importing the session from scratch.
- Re-generating files already present in the `exports/` directory.
- Asking for credentials — none are required for local pipeline work.

## Next Recommended Task
> {next_task}

## Validation Commands
```bash
cargo build
cargo test
```

## Handoff Notes
- Provider health at export time: {health}
- Source artifacts are in `raw/`.
- Full canonical session is in `normalized/canonical-session.json`.
- Target exports in `exports/` are readable context seeds, not native session files;
  native resume compatibility is not guaranteed.
"#,
        source     = ctx.source,
        targets    = targets_str,
        turn_count = turns.len(),
        objective  = objective,
        files_inspect = files_inspect,
        completed_str = completed_str,
        records    = records_section(&handoff_records),
        next_task  = next_task_str,
        health     = health_summary(ctx.provider_health, ctx.source),
    );

    std::fs::write(output_dir.join("AGENTS.md"), agents)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use serde_json::json;

    #[test]
    fn test_handoff_and_kiro_files_generated() {
        let canonical_turns = vec![
            CanonicalTurn {
                id: "turn-1".to_string(),
                role: "user".to_string(),
                content: "Build a Rust CLI tool".to_string(),
                timestamp: "2024-01-01T10:00:00Z".to_string(),
                metadata: json!({}),
                ..Default::default()
            },
            CanonicalTurn {
                id: "turn-2".to_string(),
                role: "assistant".to_string(),
                content: "Here is the implementation.".to_string(),
                timestamp: "2024-01-01T10:00:05Z".to_string(),
                metadata: json!({}),
                ..Default::default()
            },
        ];

        let targets = vec!["codex".to_string(), "kiro".to_string()];
        let ctx = HandoffContext {
            source: "claude-code",
            targets: &targets,
            repo_snapshot: None,
            provider_health: None,
        };

        let temp_dir = TempDir::new().unwrap();
        let output_dir = temp_dir.path();

        create_handoff_docs(&canonical_turns, &ctx, output_dir).unwrap();

        let handoff = fs::read_to_string(output_dir.join("HANDOFF.md")).unwrap();
        assert!(handoff.contains("Agent Airlift Handoff"));
        assert!(handoff.contains("claude-code"));
        assert!(handoff.contains("Build a Rust CLI tool"));
        assert!(handoff.contains("Resume Prompt"));

        let agents = fs::read_to_string(output_dir.join("AGENTS.md")).unwrap();
        assert!(agents.contains("Current Objective"));
        assert!(agents.contains("Build a Rust CLI tool"));
        assert!(agents.contains("Validation Commands"));
        assert!(agents.contains("cargo build"));

        // Kiro spec files via export_for_target
        let temp_dir2 = TempDir::new().unwrap();
        let output_dir2 = temp_dir2.path();
        export_for_target("kiro", &canonical_turns, output_dir2).unwrap();
        let spec_dir = output_dir2.join(".kiro/specs/agent-airlift-handoff");
        assert!(spec_dir.join("requirements.md").exists());
        assert!(spec_dir.join("design.md").exists());
        assert!(spec_dir.join("tasks.md").exists());
    }

    #[test]
    fn test_export_for_targets() {
        let canonical_turns = vec![
            CanonicalTurn {
                id: "test-1".to_string(),
                role: "user".to_string(),
                content: "Test".to_string(),
                timestamp: "2024-01-01T10:00:00Z".to_string(),
                record_type: "user".to_string(),
                canonical_sha256: "hash-1".to_string(),
                tool_results: json!([{"type": "tool_result", "content": "result"}]),
                metadata: json!({}),
                ..Default::default()
            },
        ];
        
        // Test codex export
        let temp_dir1 = TempDir::new().unwrap();
        let output_dir1 = temp_dir1.path();
        fs::create_dir_all(output_dir1).unwrap();
        export_for_target("codex", &canonical_turns, output_dir1).unwrap();
        let codex_content = fs::read_to_string(output_dir1.join("codex-like.session.jsonl")).unwrap();
        let codex_lines: Vec<Value> = codex_content
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(codex_lines[0]["resume_compatible"], false);
        assert_eq!(codex_lines[1]["id"], "test-1");
        assert_eq!(codex_lines[1]["role"], "user");
        assert_eq!(codex_lines[1]["agent_airlift_canonical"]["tool_results"][0]["content"], "result");
        
        // Test kiro export
        let temp_dir2 = TempDir::new().unwrap();
        let output_dir2 = temp_dir2.path();
        fs::create_dir_all(output_dir2).unwrap();
        export_for_target("kiro", &canonical_turns, output_dir2).unwrap();
        let kiro_content = fs::read_to_string(output_dir2.join("kiro-session.json")).unwrap();
        let kiro: Value = serde_json::from_str(&kiro_content).unwrap();
        assert_eq!(kiro["version"], "1.0");
        assert_eq!(kiro["turns"][0]["id"], "test-1");
        assert_eq!(kiro["turns"][0]["agent_airlift_canonical"]["tool_results"][0]["content"], "result");
        
        // Test opencode export
        let temp_dir3 = TempDir::new().unwrap();
        let output_dir3 = temp_dir3.path();
        fs::create_dir_all(output_dir3).unwrap();
        export_for_target("opencode", &canonical_turns, output_dir3).unwrap();
        let opencode_content = fs::read_to_string(output_dir3.join("opencode-like.session.jsonl")).unwrap();
        let opencode_lines: Vec<Value> = opencode_content
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(opencode_lines[0]["resume_compatible"], false);
        assert_eq!(opencode_lines[1]["message_id"], "test-1");
        assert_eq!(opencode_lines[1]["sender"], "user");
        assert_eq!(opencode_lines[1]["agent_airlift_canonical"]["tool_results"][0]["content"], "result");
        
        // Test invalid target
        let result = export_for_target("invalid", &canonical_turns, output_dir1);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported target"));
    }

    #[test]
    fn test_health_summary_distinguishes_signal_source_from_evaluated_provider() {
        let health = json!({
            "provider": "claude-code",
            "source": "marginlab",
            "status": "degraded",
            "reason": "High latency detected"
        });
        let summary = health_summary(Some(&health), "claude-code");
        // Signal source (marginlab) and evaluated provider (claude-code) must both appear
        assert!(summary.contains("marginlab"), "should mention signal source");
        assert!(summary.contains("claude-code"), "should mention evaluated provider");
        assert!(summary.contains("degraded"));
        // Must NOT say "marginlab is degraded" — that conflates source with evaluated provider
        assert!(!summary.contains("`marginlab` is"), "must not say marginlab is degraded");
    }

    #[test]
    fn test_next_task_suggests_validation_when_last_turn_is_assistant() {
        // Session ends with assistant reply → last user request was addressed
        let turns = vec![
            CanonicalTurn { id: "t1".into(), role: "user".into(),
                content: "Add a version flag".into(), timestamp: "".into(), metadata: json!({}), ..Default::default() },
            CanonicalTurn { id: "t2".into(), role: "assistant".into(),
                content: "Added version flag.".into(), timestamp: "".into(), metadata: json!({}), ..Default::default() },
        ];
        let task = next_task(&turns);
        assert!(task.contains("Validate"), "should suggest validation, got: {}", task);
        assert!(task.contains("Add a version flag"), "should reference the completed request");

        // Session ends with user message → still open, return it directly
        let turns_open = vec![
            CanonicalTurn { id: "t1".into(), role: "user".into(),
                content: "Add a version flag".into(), timestamp: "".into(), metadata: json!({}), ..Default::default() },
        ];
        let task_open = next_task(&turns_open);
        assert!(task_open.contains("Add a version flag"));
        assert!(!task_open.contains("Validate"), "open request should not say Validate");
    }

    #[test]
    fn test_handoff_contains_actionable_sections() {
        let turns = vec![
            CanonicalTurn { id: "u1".into(), role: "user".into(),
                content: "Add a /health endpoint".into(), timestamp: "".into(), ..Default::default() },
            CanonicalTurn { id: "a1".into(), role: "assistant".into(),
                content: "DECISION(auth-json): using axum::Json.\nRATIONALE(auth-json): matches existing handlers.\nSTATUS(auth-json): settled - do not revisit.\n$ cargo build\nerror[E0425]: cannot find function".into(),
                timestamp: "".into(), ..Default::default() },
        ];
        let targets = vec!["codex".to_string()];
        let ctx = HandoffContext { source: "claude-code", targets: &targets, repo_snapshot: None, provider_health: None };
        let dir = TempDir::new().unwrap();
        create_handoff_docs(&turns, &ctx, dir.path()).unwrap();

        let h = fs::read_to_string(dir.path().join("HANDOFF.md")).unwrap();
        for section in [
            "## Current Objective",
            "## Decisions Made",
            "## Commands Run",
            "## Errors and Risks",
            "## Open Questions",
            "## Recommended Next Actions",
            "## Resume Prompt",
            "## Resume Compatibility",
            "## Evidence / Source Artifacts",
        ] {
            assert!(h.contains(section), "HANDOFF.md missing actionable section: {}", section);
        }
        // It actually extracted the decision, command, and error from the session
        assert!(h.contains("axum::Json"));
        assert!(h.contains("cargo build"));
        assert!(h.contains("E0425"));
        assert!(h.contains("DECISION(auth-json): using axum::Json."));
        assert!(h.contains("RATIONALE(auth-json): matches existing handlers."));
        assert!(h.contains("STATUS(auth-json): settled - do not revisit."));

        let agents = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("DECISION(auth-json): using axum::Json."));
        assert!(agents.contains("RATIONALE(auth-json): matches existing handlers."));
        assert!(agents.contains("STATUS(auth-json): settled - do not revisit."));
    }

    #[test]
    fn test_kiro_specs_include_real_session_context() {
        let turns = vec![
            CanonicalTurn {
                id: "u1".into(),
                role: "user".into(),
                content: "Build real Kiro specs".into(),
                timestamp: "".into(),
                ..Default::default()
            },
            CanonicalTurn {
                id: "a1".into(),
                role: "assistant".into(),
                content: "DECISION(specs): derive specs from handoff context.\nRATIONALE: placeholders lose task state.\nSTATUS: settled.".into(),
                timestamp: "".into(),
                ..Default::default()
            },
        ];
        let dir = TempDir::new().unwrap();
        export_for_target("kiro", &turns, dir.path()).unwrap();

        let spec_dir = dir.path().join(".kiro/specs/agent-airlift-handoff");
        let requirements = fs::read_to_string(spec_dir.join("requirements.md")).unwrap();
        let design = fs::read_to_string(spec_dir.join("design.md")).unwrap();
        let tasks = fs::read_to_string(spec_dir.join("tasks.md")).unwrap();

        assert!(requirements.contains("Build real Kiro specs"));
        assert!(design.contains("DECISION(specs): derive specs"));
        assert!(design.contains("RATIONALE: placeholders lose task state."));
        assert!(tasks.contains("Review migrated session"));
    }

    #[test]
    fn test_inline_decision_pattern_still_survives() {
        let turns = vec![
            CanonicalTurn {
                id: "a1".into(),
                role: "assistant".into(),
                content: "Done. Decision: base62.".into(),
                timestamp: "".into(),
                ..Default::default()
            },
        ];
        let targets = vec!["codex".to_string()];
        let ctx = HandoffContext {
            source: "claude-code",
            targets: &targets,
            repo_snapshot: None,
            provider_health: None,
        };
        let dir = TempDir::new().unwrap();
        create_handoff_docs(&turns, &ctx, dir.path()).unwrap();

        let handoff = fs::read_to_string(dir.path().join("HANDOFF.md")).unwrap();
        assert!(handoff.contains("Decision: base62."));
    }

    #[test]
    fn test_agents_does_not_recommend_repeating_completed_work() {
        // Final user request was answered by a following assistant turn.
        let turns = vec![
            CanonicalTurn { id: "u1".into(), role: "user".into(),
                content: "Add a version flag".into(), timestamp: "".into(), ..Default::default() },
            CanonicalTurn { id: "a1".into(), role: "assistant".into(),
                content: "Added the version flag and verified the build.".into(), timestamp: "".into(), ..Default::default() },
        ];
        let targets = vec!["kiro".to_string()];
        let ctx = HandoffContext { source: "claude-code", targets: &targets, repo_snapshot: None, provider_health: None };
        let dir = TempDir::new().unwrap();
        create_handoff_docs(&turns, &ctx, dir.path()).unwrap();

        let agents = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        // Has an explicit Do Not Repeat section
        assert!(agents.contains("## Do Not Repeat"));
        // Next task is validation-oriented, not a blind repeat of the completed request
        let next_line = agents.lines()
            .skip_while(|l| !l.contains("## Next Recommended Task"))
            .nth(1)
            .unwrap_or("");
        assert!(next_line.contains("Validate"),
            "Next task should be validation-oriented, got: {}", next_line);
    }
}
