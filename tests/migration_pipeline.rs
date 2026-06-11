use serde_json::Value;
use std::fs;
use std::process::Command;

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("read json")).expect("parse json")
}

fn read_jsonl(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .expect("read jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse jsonl line"))
        .collect()
}

#[test]
fn migration_pipeline_enforces_ci_gate_invariants() {
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = env!("CARGO_BIN_EXE_agent-airlift");
    let temp = tempfile::TempDir::new().expect("tempdir");
    let out = temp.path().join("airlift-out");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");

    let output = Command::new(bin)
        .arg("migrate")
        .arg("--session")
        .arg(repo.join("examples/sessions/compact-boundary.jsonl"))
        .arg("--project")
        .arg(repo.join("examples/projects/tiny-rust-cli"))
        .arg("--out")
        .arg(&out)
        .arg("--source")
        .arg("claude-code")
        .arg("--targets")
        .arg("codex,claude-code,kiro,opencode")
        .arg("--provider-health")
        .arg("file")
        .arg("--provider-health-file")
        .arg(repo.join("examples/provider-health/degraded.marginlab.cached.json"))
        .env("HOME", &home)
        .output()
        .expect("run migration");

    assert!(
        output.status.success(),
        "migration failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !home.join(".codex").exists(),
        "native install must be opt-in"
    );
    assert!(
        !home.join(".claude").exists(),
        "native install must be opt-in"
    );

    let gate = read_json(&out.join("audit/ci-gate.json"));
    assert_eq!(gate["passed"], true);
    assert_eq!(gate["checks"]["accounting_balanced"], true);
    assert_eq!(gate["checks"]["dropped_field_budget_clean"], true);
    assert_eq!(gate["checks"]["hashes_present"], true);
    assert_eq!(gate["checks"]["replay_hashes_match"], true);
    assert_eq!(gate["checks"]["exports_match_canonical"], true);

    let diagnostics = read_json(&out.join("audit/import-diagnostics.json"));
    assert_eq!(diagnostics["accounting_balanced"], true);
    assert_eq!(
        diagnostics["lines_read"].as_u64().unwrap(),
        diagnostics["mapped_records"].as_u64().unwrap()
            + diagnostics["intentionally_skipped_records"]
                .as_u64()
                .unwrap()
            + diagnostics["malformed_records"].as_u64().unwrap()
    );

    let dropped = read_json(&out.join("audit/dropped-fields.json"));
    assert!(dropped["unapproved_drops"].as_array().unwrap().is_empty());

    let canonical = read_json(&out.join("normalized/canonical-session.json"));
    let canonical_turns = canonical.as_array().expect("canonical array");
    assert!(
        !canonical_turns.is_empty(),
        "canonical session must not be empty"
    );
    for turn in canonical_turns {
        assert_eq!(turn["raw_sha256"].as_str().unwrap().len(), 64);
        assert_eq!(turn["canonical_sha256"].as_str().unwrap().len(), 64);
    }

    let replay = read_jsonl(&out.join("replay/agent-airlift.session.jsonl"));
    assert_eq!(replay.len(), canonical_turns.len());
    for (line, turn) in replay.iter().zip(canonical_turns.iter()) {
        assert_eq!(line["canonical_sha256"], turn["canonical_sha256"]);
        assert_eq!(
            line["canonical"]["canonical_sha256"],
            turn["canonical_sha256"]
        );
    }

    assert!(out.join("exports/HANDOFF.md").metadata().unwrap().len() > 0);
    assert!(out.join("exports/AGENTS.md").metadata().unwrap().len() > 0);
    assert!(out
        .join("exports/native/codex")
        .read_dir()
        .expect("codex native export")
        .next()
        .is_some());
    assert!(out
        .join("exports/native/claude-code")
        .read_dir()
        .expect("claude native export")
        .next()
        .is_some());
}

#[test]
fn migration_pipeline_preserves_real_claude_compact_summary_in_exports() {
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = env!("CARGO_BIN_EXE_agent-airlift");
    let temp = tempfile::TempDir::new().expect("tempdir");
    let out = temp.path().join("airlift-out");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");

    let output = Command::new(bin)
        .arg("migrate")
        .arg("--session")
        .arg(repo.join("examples/sessions/claude-compact-boundary-realistic.jsonl"))
        .arg("--project")
        .arg(repo.join("examples/projects/tiny-rust-cli"))
        .arg("--out")
        .arg(&out)
        .arg("--source")
        .arg("claude-code")
        .arg("--targets")
        .arg("codex")
        .arg("--provider-health")
        .arg("none")
        .env("HOME", &home)
        .output()
        .expect("run migration");

    assert!(
        output.status.success(),
        "migration failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let canonical = read_json(&out.join("normalized/canonical-session.json"));
    let summary = canonical
        .as_array()
        .unwrap()
        .iter()
        .find(|turn| turn["id"] == "u-real-compact-summary")
        .expect("summary turn");
    assert_eq!(summary["role"], "summary");
    assert_eq!(summary["record_type"], "compact_summary");
    assert!(summary["content"]
        .as_str()
        .unwrap()
        .contains("DECISION(api-v2): Keep API v2"));

    let handoff = fs::read_to_string(out.join("exports/HANDOFF.md")).expect("handoff");
    assert!(handoff.contains("DECISION(api-v2): Keep API v2"));
    let codex_export =
        fs::read_to_string(out.join("exports/codex-like.session.jsonl")).expect("codex export");
    assert!(codex_export.contains("compact_summary"));
    assert!(codex_export.contains("Continue preserving compaction summaries"));
}

#[test]
fn migration_pipeline_exports_canonical_sidecars_idempotently_for_all_targets() {
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = env!("CARGO_BIN_EXE_agent-airlift");
    let temp = tempfile::TempDir::new().expect("tempdir");
    let out = temp.path().join("airlift-out");
    let home = temp.path().join("home");
    fs::create_dir_all(&home).expect("home");

    let output = Command::new(bin)
        .arg("migrate")
        .arg("--session")
        .arg(repo.join("examples/sessions/claude-compact-boundary-realistic.jsonl"))
        .arg("--project")
        .arg(repo.join("examples/projects/tiny-rust-cli"))
        .arg("--out")
        .arg(&out)
        .arg("--source")
        .arg("claude-code")
        .arg("--targets")
        .arg("codex,claude-code,kiro,opencode")
        .arg("--provider-health")
        .arg("none")
        .env("HOME", &home)
        .output()
        .expect("run migration");

    assert!(
        output.status.success(),
        "migration failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let canonical = read_json(&out.join("normalized/canonical-session.json"));
    let canonical_turns = canonical.as_array().expect("canonical array").clone();

    let codex = read_jsonl(&out.join("exports/codex-like.session.jsonl"));
    let claude = read_jsonl(&out.join("exports/claude-code-like.session.jsonl"));
    let opencode = read_jsonl(&out.join("exports/opencode-like.session.jsonl"));
    let kiro = read_json(&out.join("exports/kiro-session.json"));

    for exported in [&codex[1..], &claude[1..], &opencode[1..]] {
        let sidecars: Vec<Value> = exported
            .iter()
            .map(|line| line["agent_airlift_canonical"].clone())
            .collect();
        assert_eq!(sidecars, canonical_turns);
    }

    let kiro_sidecars: Vec<Value> = kiro["turns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|turn| turn["agent_airlift_canonical"].clone())
        .collect();
    assert_eq!(kiro_sidecars, canonical_turns);

    let gate = read_json(&out.join("audit/ci-gate.json"));
    assert_eq!(gate["checks"]["exports_match_canonical"], true);
}
