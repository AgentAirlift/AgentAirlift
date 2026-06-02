use std::fs;
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
