use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn chmod_executable(path: &std::path::Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod");
}

fn write_fake_agent_airlift(bin_dir: &std::path::Path, log_path: &std::path::Path) -> PathBuf {
    fs::create_dir_all(bin_dir).expect("bin dir");
    let fake = bin_dir.join("agent-airlift");
    fs::write(
        &fake,
        r#"#!/usr/bin/env sh
printf '%s\n' "$*" >> "$AIRLIFT_ARGS_LOG"
exit 0
"#,
    )
    .expect("fake agent-airlift");
    chmod_executable(&fake);
    fs::write(log_path, "").expect("empty args log");
    fake
}

fn touch_at(path: &std::path::Path, timestamp: &str) {
    let status = Command::new("touch")
        .arg("-t")
        .arg(timestamp)
        .arg(path)
        .status()
        .expect("touch");
    assert!(status.success(), "touch failed for {}", path.display());
}

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

    assert!(
        !fs::symlink_metadata(prompts_dir.join("airlift-check.md"))
            .expect("check prompt metadata")
            .file_type()
            .is_symlink(),
        "normal installs should copy prompts, not link them to the repo"
    );
    assert!(
        !fs::symlink_metadata(prompts_dir.join("airlift-migrate.md"))
            .expect("migrate prompt metadata")
            .file_type()
            .is_symlink(),
        "normal installs should copy prompts, not link them to the repo"
    );
    assert_eq!(
        fs::read_to_string(prompts_dir.join("airlift-check.md")).expect("check prompt"),
        fs::read_to_string(repo.join("plugins/codex/prompts/airlift-check.md"))
            .expect("source prompt")
    );
    assert_eq!(
        fs::read_to_string(prompts_dir.join("airlift-migrate.md")).expect("migrate prompt"),
        fs::read_to_string(repo.join("plugins/codex/prompts/airlift-migrate.md"))
            .expect("source prompt")
    );

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
fn install_codex_never_overwrites_existing_backup() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::TempDir::new().expect("tempdir");
    let codex_home = temp.path().join("codex-home");
    let prompts_dir = codex_home.join("prompts");
    fs::create_dir_all(&prompts_dir).expect("create prompts");
    fs::write(
        prompts_dir.join("airlift-check.md"),
        "current user prompt\n",
    )
    .expect("old prompt");
    fs::write(prompts_dir.join("airlift-check.md.bak"), "prior backup\n").expect("prior backup");

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

    assert_eq!(
        fs::read_to_string(prompts_dir.join("airlift-check.md.bak")).expect("original backup"),
        "prior backup\n",
        "installer must not overwrite an existing backup"
    );
    assert_eq!(
        fs::read_to_string(prompts_dir.join("airlift-check.md.bak.1")).expect("new backup"),
        "current user prompt\n"
    );
}

#[test]
fn codex_migrate_selects_session_for_current_project() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    let other_project = temp.path().join("other-project");
    let sessions_dir = home.join(".codex/sessions/2026/06/09");
    let bin_dir = temp.path().join("bin");
    let log_path = temp.path().join("args.log");
    fs::create_dir_all(&project).expect("project");
    fs::create_dir_all(&other_project).expect("other project");
    fs::create_dir_all(&sessions_dir).expect("sessions");
    let project_session = sessions_dir.join("rollout-2026-06-09T12-00-00-project.jsonl");
    let other_session = sessions_dir.join("rollout-2026-06-09T12-01-00-other.jsonl");
    fs::write(
        &project_session,
        format!(
            r#"{{"timestamp":"2026-06-09T12:00:00Z","type":"session_meta","payload":{{"id":"project","timestamp":"2026-06-09T12:00:00Z","cwd":"{}"}}}}"#,
            project.display()
        ),
    )
    .expect("project session");
    fs::write(
        &other_session,
        format!(
            r#"{{"timestamp":"2026-06-09T12:01:00Z","type":"session_meta","payload":{{"id":"other","timestamp":"2026-06-09T12:01:00Z","cwd":"{}"}}}}"#,
            other_project.display()
        ),
    )
    .expect("other session");
    touch_at(&project_session, "202606091200.00");
    touch_at(&other_session, "202606091201.00");
    let fake = write_fake_agent_airlift(&bin_dir, &log_path);

    let output = Command::new("bash")
        .arg(repo.join("plugins/codex/scripts/migrate.sh"))
        .arg("claude-code")
        .env("HOME", &home)
        .env("AGENT_AIRLIFT_BIN", &fake)
        .env("AIRLIFT_ARGS_LOG", &log_path)
        .current_dir(&project)
        .output()
        .expect("run codex migrate");

    assert!(
        output.status.success(),
        "migrate failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let log = fs::read_to_string(&log_path).expect("args log");
    assert!(
        log.contains(project_session.to_str().unwrap()),
        "args: {log}"
    );
    assert!(
        !log.contains(other_session.to_str().unwrap()),
        "args: {log}"
    );
    assert!(
        log.contains(
            repo.join("examples/provider-health/degraded.marginlab.cached.codex.json")
                .to_str()
                .unwrap()
        ),
        "default Codex cache path should be repo-rooted, args: {log}"
    );
}

#[test]
fn codex_migrate_fails_closed_without_live_session() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    let bin_dir = temp.path().join("bin");
    let log_path = temp.path().join("args.log");
    fs::create_dir_all(&project).expect("project");
    let fake = write_fake_agent_airlift(&bin_dir, &log_path);

    let output = Command::new("bash")
        .arg(repo.join("plugins/codex/scripts/migrate.sh"))
        .arg("claude-code")
        .env("HOME", &home)
        .env("AGENT_AIRLIFT_BIN", &fake)
        .env("AIRLIFT_ARGS_LOG", &log_path)
        .current_dir(&project)
        .output()
        .expect("run codex migrate");

    assert!(
        !output.status.success(),
        "script should fail without a live session"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("AIRLIFT_ALLOW_FIXTURE_FALLBACK=1"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&log_path).expect("args log"), "");
}

#[test]
fn claude_migrate_canonicalizes_cwd_before_selecting_session() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let project = temp.path().join("project-real");
    let link = temp.path().join("project-link");
    let bin_dir = temp.path().join("bin");
    let log_path = temp.path().join("args.log");
    fs::create_dir_all(&project).expect("project");
    std::os::unix::fs::symlink(&project, &link).expect("project symlink");
    let canonical = project.canonicalize().expect("canonical project");
    let project_key = canonical.to_string_lossy().replace('/', "-");
    let session_dir = home.join(".claude/projects").join(project_key);
    fs::create_dir_all(&session_dir).expect("session dir");
    let session = session_dir.join("session.jsonl");
    fs::write(&session, "{}\n").expect("session");
    let fake = write_fake_agent_airlift(&bin_dir, &log_path);

    let command = format!("cd \"$1\" && \"$2\" codex");
    let output = Command::new("bash")
        .arg("-c")
        .arg(command)
        .arg("airlift-test")
        .arg(&link)
        .arg(repo.join("plugins/claude/scripts/migrate.sh"))
        .env("HOME", &home)
        .env("AGENT_AIRLIFT_BIN", &fake)
        .env("AIRLIFT_ARGS_LOG", &log_path)
        .output()
        .expect("run claude migrate");

    assert!(
        output.status.success(),
        "migrate failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let log = fs::read_to_string(&log_path).expect("args log");
    assert!(log.contains(session.to_str().unwrap()), "args: {log}");
    assert!(
        log.contains(
            repo.join("examples/provider-health/degraded.marginlab.cached.json")
                .to_str()
                .unwrap()
        ),
        "default Claude cache path should be repo-rooted, args: {log}"
    );
}

#[test]
fn migrate_fixture_fallback_paths_are_repo_rooted() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    let bin_dir = temp.path().join("bin");
    let codex_log = temp.path().join("codex-args.log");
    let claude_log = temp.path().join("claude-args.log");
    fs::create_dir_all(&project).expect("project");

    let fake = write_fake_agent_airlift(&bin_dir, &codex_log);
    let output = Command::new("bash")
        .arg(repo.join("plugins/codex/scripts/migrate.sh"))
        .arg("claude-code")
        .env("HOME", &home)
        .env("AGENT_AIRLIFT_BIN", &fake)
        .env("AIRLIFT_ARGS_LOG", &codex_log)
        .env("AIRLIFT_ALLOW_FIXTURE_FALLBACK", "1")
        .current_dir(&project)
        .output()
        .expect("run codex migrate");
    assert!(
        output.status.success(),
        "codex migrate failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let codex_args = fs::read_to_string(&codex_log).expect("codex args");
    assert!(
        codex_args.contains(repo.join("examples/sessions/codex-realistic.jsonl").to_str().unwrap()),
        "codex fixture path should be repo-rooted, args: {codex_args}"
    );

    fs::write(&claude_log, "").expect("clear claude log");
    let output = Command::new("bash")
        .arg(repo.join("plugins/claude/scripts/migrate.sh"))
        .arg("codex")
        .env("HOME", &home)
        .env("AGENT_AIRLIFT_BIN", &fake)
        .env("AIRLIFT_ARGS_LOG", &claude_log)
        .env("AIRLIFT_ALLOW_FIXTURE_FALLBACK", "1")
        .current_dir(&project)
        .output()
        .expect("run claude migrate");
    assert!(
        output.status.success(),
        "claude migrate failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let claude_args = fs::read_to_string(&claude_log).expect("claude args");
    assert!(
        claude_args.contains(
            repo.join("examples/sessions/claude-code-realistic.jsonl")
                .to_str()
                .unwrap()
        ),
        "claude fixture path should be repo-rooted, args: {claude_args}"
    );
}

#[test]
fn claude_migrate_fails_closed_without_live_session() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::TempDir::new().expect("tempdir");
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    let bin_dir = temp.path().join("bin");
    let log_path = temp.path().join("args.log");
    fs::create_dir_all(&project).expect("project");
    let fake = write_fake_agent_airlift(&bin_dir, &log_path);

    let output = Command::new("bash")
        .arg(repo.join("plugins/claude/scripts/migrate.sh"))
        .arg("codex")
        .env("HOME", &home)
        .env("AGENT_AIRLIFT_BIN", &fake)
        .env("AIRLIFT_ARGS_LOG", &log_path)
        .current_dir(&project)
        .output()
        .expect("run claude migrate");

    assert!(
        !output.status.success(),
        "script should fail without a live session"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("AIRLIFT_ALLOW_FIXTURE_FALLBACK=1"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&log_path).expect("args log"), "");
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
    chmod_executable(&fake_claude);

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
