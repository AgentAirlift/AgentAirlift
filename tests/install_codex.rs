use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn install_codex_replaces_prompts_and_installs_skills() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::TempDir::new().expect("tempdir");
    let codex_home = temp.path().join("codex-home");
    let prompts_dir = codex_home.join("prompts");
    fs::create_dir_all(&prompts_dir).expect("create prompts");
    fs::write(prompts_dir.join("airlift-check.md"), "old check prompt\n").expect("old prompt");

    let output = Command::new("python3")
        .arg(repo.join("scripts/agent-airlift"))
        .arg("install-codex")
        .arg("--codex-home")
        .arg(&codex_home)
        .arg("--skip-build")
        .current_dir(&repo)
        .output()
        .expect("run installer");

    assert!(
        output.status.success(),
        "installer failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let check_link = fs::read_link(prompts_dir.join("airlift-check.md")).expect("check symlink");
    let migrate_link = fs::read_link(prompts_dir.join("airlift-migrate.md")).expect("migrate symlink");
    assert_eq!(check_link, repo.join("plugins/codex/prompts/airlift-check.md"));
    assert_eq!(migrate_link, repo.join("plugins/codex/prompts/airlift-migrate.md"));

    let backup = fs::read_to_string(prompts_dir.join("airlift-check.md.bak")).expect("backup");
    assert_eq!(backup, "old check prompt\n");

    let skill = fs::read_to_string(codex_home.join("skills/airlift-check/SKILL.md"))
        .expect("airlift-check skill");
    assert!(skill.contains("Marginlab"));
    assert!(skill.contains("bash plugins/codex/scripts/check.sh"));

    let migrate_skill = fs::read_to_string(codex_home.join("skills/airlift-migrate/SKILL.md"))
        .expect("airlift-migrate skill");
    assert!(migrate_skill.contains("bash plugins/codex/scripts/migrate.sh claude-code"));
}

#[test]
fn install_claude_reinstalls_plugin_via_claude_cli() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::TempDir::new().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    let home_dir = temp.path().join("home");
    let log_path = temp.path().join("claude.log");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    fs::create_dir_all(&home_dir).expect("home dir");

    let fake_claude = bin_dir.join("claude");
    fs::write(
        &fake_claude,
        r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> "$CLAUDE_LOG"
exit 0
"#,
    )
    .expect("fake claude");
    let mut permissions = fs::metadata(&fake_claude).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_claude, permissions).expect("chmod");

    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let output = Command::new("python3")
        .arg(repo.join("scripts/agent-airlift"))
        .arg("install-claude")
        .arg("--skip-build")
        .env("PATH", path)
        .env("HOME", &home_dir)
        .env("CLAUDE_LOG", &log_path)
        .current_dir(&repo)
        .output()
        .expect("run installer");

    assert!(
        output.status.success(),
        "installer failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let log = fs::read_to_string(log_path).expect("claude log");
    let expected = format!(
        "plugin validate {plugin}\n\
         plugin marketplace add {marketplace}\n\
         plugin uninstall agent-airlift@agent-airlift-local --scope user --keep-data -y\n\
         plugin install agent-airlift@agent-airlift-local --scope user\n",
        plugin = repo.join("plugins/claude").display(),
        marketplace = repo.join("plugins").display()
    );
    assert_eq!(log, expected);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Installed Claude plugin: agent-airlift@agent-airlift-local"));
}
