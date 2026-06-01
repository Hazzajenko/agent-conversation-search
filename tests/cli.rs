//! End-to-end tests driving the `ccsearch` binary as a user would.

use assert_cmd::Command;
use std::fs;

/// Build a fixture Store under `store_root` containing one Session for the given
/// working directory, then return a `Command` ready to run from that cwd with
/// `--claude-dir` pointed at the fixture.
fn ccsearch_in(cwd: &std::path::Path, store_root: &std::path::Path, session_lines: &str) -> Command {
    let projects = store_root.join("projects");
    let encoded = ccsearch::encode_project_dir(&cwd.to_string_lossy());
    let project_dir = projects.join(encoded);
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join("session.jsonl"), session_lines).unwrap();

    let mut cmd = Command::cargo_bin("ccsearch").unwrap();
    cmd.current_dir(cwd).arg("--claude-dir").arg(store_root);
    cmd
}

#[test]
fn finds_a_match_in_the_current_projects_conversations() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"how to use tokio select"}}"#,
    );

    cmd.arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("how to use tokio select"));
}

#[test]
fn reports_cleanly_when_there_are_no_matches() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"nothing relevant here"}}"#,
    );

    cmd.arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches"));
}
