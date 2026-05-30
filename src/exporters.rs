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
        "kiro" => export_kiro(turns, output_dir),
        "opencode" => export_opencode(turns, output_dir),
        _ => Err(anyhow::anyhow!("Unsupported target: {}", target)),
    }
}

fn export_codex(turns: &[CanonicalTurn], output_dir: &std::path::Path) -> anyhow::Result<()> {
    // Leading meta line documents provenance. This is a readable, deterministic
    // Agent Airlift export — NOT a native Codex rollout file. Native `codex resume`
    // compatibility is not guaranteed; use this to seed a fresh session.
    let mut lines = vec![json!({
        "_meta": "agent-airlift-export",
        "format": "codex-like",
        "resume_compatible": false,
        "note": "Readable export for resuming context, not a native Codex session file."
    }).to_string()];
    for turn in turns {
        let line = json!({
            "id": turn.id,
            "role": turn.role,
            "content": turn.content,
            "timestamp": turn.timestamp,
            "tool_calls": turn.tool_calls,
            "source": turn.source,
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
    std::fs::write(spec_dir.join("requirements.md"), "# Requirements\n\nMigrated session requirements.")?;
    std::fs::write(spec_dir.join("design.md"), "# Design\n\nSession design documentation.")?;
    std::fs::write(spec_dir.join("tasks.md"), "# Tasks\n\n1. Review migrated session\n2. Test functionality")?;
    
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
            // `source` = signal origin (apify, cached-apify, file, mock)
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

/// Lines from assistant turns that look like decisions ("Decision:" anywhere).
fn extract_decisions(turns: &[CanonicalTurn]) -> Vec<String> {
    let mut out = Vec::new();
    for t in turns.iter().filter(|t| t.role == "assistant") {
        for line in t.content.lines() {
            if let Some(idx) = line.find("Decision:").or_else(|| line.find("decision:")) {
                let d = line[idx..].trim();
                let entry = format!("- {}", d);
                if !out.contains(&entry) { out.push(entry); }
            }
        }
    }
    if out.is_empty() {
        out.push("- No explicit Decision: lines detected. Review assistant turns for implicit choices.".into());
    }
    out
}

/// Commands from assistant content heuristics AND structured Bash tool calls.
fn extract_commands(turns: &[CanonicalTurn]) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |cmd: &str, out: &mut Vec<String>| {
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
    let decisions   = extract_decisions(turns);
    let commands    = extract_commands(turns);
    let errors      = extract_errors(turns);
    let targets_str = ctx.targets.join(", ");
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
{decisions}

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
        objective  = objective,
        turn_count = turns.len(),
        last_user  = &last_user[..last_user.len().min(200)],
        files_section = files_section,
        decisions  = decisions.join("\n"),
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
- Do not call Box or Apify APIs — they are not yet implemented.
- Keep changes deterministic and local unless explicitly instructed otherwise.
- Preserve unknown fields in any JSONL you read or write.

## Files to Inspect First
{files_inspect}

## Work Already Completed
{completed_str}

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
        assert!(codex_content.contains("test-1"));
        assert!(codex_content.contains("user"));
        
        // Test kiro export
        let temp_dir2 = TempDir::new().unwrap();
        let output_dir2 = temp_dir2.path();
        fs::create_dir_all(output_dir2).unwrap();
        export_for_target("kiro", &canonical_turns, output_dir2).unwrap();
        let kiro_content = fs::read_to_string(output_dir2.join("kiro-session.json")).unwrap();
        assert!(kiro_content.contains("test-1"));
        assert!(kiro_content.contains("version"));
        
        // Test opencode export
        let temp_dir3 = TempDir::new().unwrap();
        let output_dir3 = temp_dir3.path();
        fs::create_dir_all(output_dir3).unwrap();
        export_for_target("opencode", &canonical_turns, output_dir3).unwrap();
        let opencode_content = fs::read_to_string(output_dir3.join("opencode-like.session.jsonl")).unwrap();
        assert!(opencode_content.contains("test-1"));
        assert!(opencode_content.contains("sender"));
        
        // Test invalid target
        let result = export_for_target("invalid", &canonical_turns, output_dir1);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported target"));
    }

    #[test]
    fn test_health_summary_distinguishes_signal_source_from_evaluated_provider() {
        let health = json!({
            "provider": "claude-code",
            "source": "apify",
            "status": "degraded",
            "reason": "High latency detected"
        });
        let summary = health_summary(Some(&health), "claude-code");
        // Signal source (apify) and evaluated provider (claude-code) must both appear
        assert!(summary.contains("apify"), "should mention signal source");
        assert!(summary.contains("claude-code"), "should mention evaluated provider");
        assert!(summary.contains("degraded"));
        // Must NOT say "apify is degraded" — that conflates source with evaluated provider
        assert!(!summary.contains("`apify` is"), "must not say apify is degraded");
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
                content: "Decision: using axum::Json.\n$ cargo build\nerror[E0425]: cannot find function".into(),
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