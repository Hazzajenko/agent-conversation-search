//! End-to-end tests driving the `ccsearch` binary as a user would.

use assert_cmd::Command;
use predicates::prelude::*;
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

/// Plant a Session directly under a named Project directory in the Store.
fn plant_project(store_root: &std::path::Path, project_name: &str, session_lines: &str) {
    let dir = store_root.join("projects").join(project_name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("session.jsonl"), session_lines).unwrap();
}

#[test]
fn all_flag_searches_projects_other_than_the_current_one() {
    let workdir = tempfile::tempdir().unwrap(); // cwd has no Project of its own
    let store = tempfile::tempdir().unwrap();
    plant_project(
        store.path(),
        "E--projects-other",
        r#"{"type":"user","message":{"role":"user","content":"run the diesel migration"}}"#,
    );

    Command::cargo_bin("ccsearch")
        .unwrap()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--all")
        .arg("diesel")
        .assert()
        .success()
        .stdout(predicates::str::contains("run the diesel migration"));
}

#[test]
fn project_flag_targets_a_named_project_by_substring() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    plant_project(
        store.path(),
        "E--projects-games-creature-game",
        r#"{"type":"user","message":{"role":"user","content":"bean asset pipeline"}}"#,
    );
    plant_project(
        store.path(),
        "C--hacking-jplag",
        r#"{"type":"user","message":{"role":"user","content":"bean counter unrelated"}}"#,
    );

    Command::cargo_bin("ccsearch")
        .unwrap()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--project")
        .arg("creature")
        .arg("bean")
        .assert()
        .success()
        .stdout(predicates::str::contains("bean asset pipeline"))
        .stdout(predicates::str::contains("bean counter unrelated").not());
}

#[test]
fn regex_flag_matches_the_query_as_a_pattern() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"reading the borrow checker docs"}}"#,
    );

    cmd.arg("--regex")
        .arg("bo+rrow")
        .assert()
        .success()
        .stdout(predicates::str::contains("reading the borrow checker docs"));
}

#[test]
fn case_sensitive_flag_excludes_a_wrong_case_match() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"the BORROW checker"}}"#,
    );

    cmd.arg("--case-sensitive")
        .arg("borrow")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches"));
}

#[test]
fn an_invalid_regex_exits_non_zero_with_a_readable_error() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"anything"}}"#,
    );

    cmd.arg("--regex")
        .arg("foo(bar")
        .assert()
        .failure()
        .stderr(predicates::str::contains("ccsearch:"))
        .stderr(predicates::str::contains("regex"));
}

#[test]
fn thinking_flag_includes_thinking_blocks() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"pondering the zylophone problem"}]}}"#,
    );

    cmd.arg("--thinking")
        .arg("zylophone")
        .assert()
        .success()
        .stdout(predicates::str::contains("zylophone"))
        .stdout(predicates::str::contains("thinking"));
}

#[test]
fn tools_flag_includes_tool_results() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"the zylophone build log"}]}}"#,
    );

    cmd.arg("--tools")
        .arg("zylophone")
        .assert()
        .success()
        .stdout(predicates::str::contains("zylophone"))
        .stdout(predicates::str::contains("tool"));
}

#[test]
fn all_content_flag_includes_tool_calls() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"zylophone --tune"}}]}}"#,
    );

    cmd.arg("--all-content")
        .arg("zylophone")
        .assert()
        .success()
        .stdout(predicates::str::contains("zylophone"))
        .stdout(predicates::str::contains("tool"));
}

#[test]
fn max_per_session_flag_caps_matches_and_notes_the_rest() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"zebra one"}}"#,
        r#"{"type":"user","message":{"role":"user","content":"zebra two"}}"#,
        r#"{"type":"user","message":{"role":"user","content":"zebra three"}}"#,
    ]
    .join("\n");
    let mut cmd = ccsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("-m")
        .arg("1")
        .arg("zebra")
        .assert()
        .success()
        .stdout(predicates::str::contains("+2 more"));
}

#[test]
fn files_flag_prints_only_the_matching_path() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"how to use tokio select"}}"#,
    );

    cmd.arg("-l")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("session.jsonl"))
        .stdout(predicates::str::contains("how to use tokio select").not());
}

#[test]
fn no_color_or_piped_output_has_no_ansi_codes() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"please highlight this"}}"#,
    );

    cmd.env_remove("CLICOLOR_FORCE")
        .env("NO_COLOR", "1")
        .arg("highlight")
        .assert()
        .success()
        .stdout(predicates::str::contains("highlight"))
        .stdout(predicates::str::contains("\u{1b}[").not());
}

#[test]
fn color_is_emitted_when_forced() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let mut cmd = ccsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"please highlight this"}}"#,
    );

    cmd.env_remove("NO_COLOR")
        .env("CLICOLOR_FORCE", "1")
        .arg("highlight")
        .assert()
        .success()
        .stdout(predicates::str::contains("\u{1b}["));
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
