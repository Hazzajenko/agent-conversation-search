//! End-to-end tests driving the `agsearch` binary as a user would.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

/// A fresh temp directory under the *canonical* system temp path.
///
/// On macOS `$TMPDIR` lives under `/var/folders`, a symlink to `/private/var`.
/// The binary reports its cwd via `getcwd`, which returns the resolved path,
/// so a fixture keyed on the symlinked path would never match the Project the
/// binary derives. Creating the directory inside the canonical root keeps both
/// sides identical on every platform.
fn tempdir() -> std::io::Result<tempfile::TempDir> {
    let root = std::env::temp_dir();
    // Windows `canonicalize` adds a `\?\` verbatim prefix, which nothing
    // else in the pipeline produces, so only resolve symlinks on Unix.
    #[cfg(not(windows))]
    let root = root.canonicalize().unwrap_or(root);
    tempfile::tempdir_in(root)
}

fn agsearch_command() -> Command {
    let mut command = Command::cargo_bin("agsearch").unwrap();
    command.env("CODEX_HOME", "__agsearch_test_missing_codex_store__");
    command.env("XDG_DATA_HOME", "__agsearch_test_missing_data_home__");
    // Isolate current-context detection from the developer's real Harness.
    command
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("CODEX_SESSION_ID")
        .env_remove("CODEX_THREAD_ID");
    command
}

/// Build a fixture Store under `store_root` containing one Session for the given
/// working directory, then return a `Command` ready to run from that cwd with
/// `--claude-dir` pointed at the fixture.
fn agsearch_in(
    cwd: &std::path::Path,
    store_root: &std::path::Path,
    session_lines: &str,
) -> Command {
    let projects = store_root.join("projects");
    let encoded = agsearch::encode_project_dir(&cwd.to_string_lossy());
    let project_dir = projects.join(encoded);
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join("session.jsonl"), session_lines).unwrap();

    let mut cmd = agsearch_command();
    cmd.current_dir(cwd).arg("--claude-dir").arg(store_root);
    cmd
}

#[test]
fn finds_a_match_in_the_current_projects_conversations() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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

/// Plant a Claude Session file named by `session_id` under the cwd's Project.
fn plant_claude_session(
    store_root: &std::path::Path,
    cwd: &std::path::Path,
    session_id: &str,
    session_lines: &str,
) -> std::path::PathBuf {
    let encoded = agsearch::encode_project_dir(&cwd.to_string_lossy());
    let dir = store_root.join("projects").join(encoded);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{session_id}.jsonl"));
    fs::write(&path, session_lines).unwrap();
    path
}

fn plant_claude_subagent(
    store_root: &std::path::Path,
    cwd: &std::path::Path,
    parent_id: &str,
    worker_id: &str,
    session_lines: &str,
) -> std::path::PathBuf {
    let encoded = agsearch::encode_project_dir(&cwd.to_string_lossy());
    let dir = store_root
        .join("projects")
        .join(encoded)
        .join(parent_id)
        .join("subagents");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{worker_id}.jsonl"));
    fs::write(&path, session_lines).unwrap();
    path
}

fn plant_codex_session(
    codex_dir: &std::path::Path,
    id: &str,
    cwd: &std::path::Path,
    thread_source: &str,
    records: &[&str],
) -> std::path::PathBuf {
    let meta = serde_json::json!({
        "timestamp": "2026-08-28T10:00:00.000Z",
        "type": "session_meta",
        "payload": {
            "id": id,
            "cwd": cwd.to_string_lossy(),
            "thread_source": thread_source,
            "agent_nickname": if thread_source == "subagent" {
                serde_json::Value::String("fixture-worker".into())
            } else {
                serde_json::Value::Null
            },
            "source": if thread_source == "subagent" {
                serde_json::json!({"subagent": {"thread_spawn": {"agent_nickname": "nested-worker"}}})
            } else {
                serde_json::json!("cli")
            }
        }
    });
    write_codex_rollout(codex_dir, id, meta, records)
}

fn plant_codex_title(codex_dir: &std::path::Path, id: &str, title: &str) {
    let path = codex_dir.join("session_index.jsonl");
    let line = serde_json::json!({"id": id, "thread_name": title}).to_string();
    fs::write(path, format!("{line}\n")).unwrap();
}

fn plant_codex_subagent(
    codex_dir: &std::path::Path,
    id: &str,
    cwd: &std::path::Path,
    parent_id: &str,
    records: &[&str],
) -> std::path::PathBuf {
    let meta = serde_json::json!({
        "timestamp": "2026-08-28T10:00:00.000Z",
        "type": "session_meta",
        "payload": {
            "id": id,
            "cwd": cwd.to_string_lossy(),
            "thread_source": "subagent",
            "parent_thread_id": parent_id,
            "agent_nickname": "fixture-worker",
            "source": {"subagent": {"thread_spawn": {"agent_nickname": "nested-worker", "parent_thread_id": parent_id}}}
        }
    });
    write_codex_rollout(codex_dir, id, meta, records)
}

fn write_codex_rollout(
    codex_dir: &std::path::Path,
    id: &str,
    meta: serde_json::Value,
    records: &[&str],
) -> std::path::PathBuf {
    let dir = codex_dir
        .join("sessions")
        .join("2026")
        .join("08")
        .join("28");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("rollout-2026-08-28T10-00-00-{id}.jsonl"));
    let text = std::iter::once(meta.to_string())
        .chain(records.iter().map(|record| (*record).to_string()))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn default_search_finds_codex_prompts_and_replies_in_the_current_project() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    plant_codex_session(
        codex.path(),
        "c0de0001-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex needle prompt"}]}}"#,
            r#"{"timestamp":"2026-08-28T10:02:00.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"codex needle reply"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("needle")
        .assert()
        .success()
        .stdout(predicates::str::contains("codex · c0de0001"))
        .stdout(predicates::str::contains("codex needle prompt"))
        .stdout(predicates::str::contains("codex needle reply"));
}

#[test]
fn search_spans_both_stores_and_harness_narrows_to_codex() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    plant_project(
        claude.path(),
        &agsearch::encode_project_dir(&workdir.path().to_string_lossy()),
        r#"{"type":"user","message":{"role":"user","content":"shared needle from Claude"}}"#,
    );
    plant_codex_session(
        codex.path(),
        "c0de0002-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"shared needle from Codex"}]}}"#,
        ],
    );

    let command = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--codex-dir")
            .arg(codex.path())
            .arg("needle");
        command
    };

    command()
        .assert()
        .success()
        .stdout(predicates::str::contains("claude · "))
        .stdout(predicates::str::contains("codex · c0de0002"));
    command()
        .arg("--harness")
        .arg("codex")
        .assert()
        .success()
        .stdout(predicates::str::contains("codex · c0de0002"))
        .stdout(predicates::str::contains("from Claude").not());
}

#[test]
fn codex_subagents_are_excluded_and_forked_sessions_remain_independent() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let record = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"fork marker"}]}}"#;
    plant_codex_session(
        codex.path(),
        "c0de0003-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[record],
    );
    plant_codex_session(
        codex.path(),
        "c0de0004-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[record],
    );
    plant_codex_session(
        codex.path(),
        "c0de0005-0000-0000-0000-000000000000",
        workdir.path(),
        "subagent",
        &[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"subagent marker"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("c0de0003"))
        .stdout(predicates::str::contains("c0de0004"))
        .stdout(predicates::str::contains("c0de0005").not());
}

#[test]
fn include_subagents_makes_workers_searchable_listable_and_showable() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    plant_codex_session(
        codex.path(),
        "c0de0015-0000-0000-0000-000000000000",
        workdir.path(),
        "subagent",
        &[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"worker-only marker"}]}}"#,
        ],
    );
    let command = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(workdir.path().join("missing-claude"))
            .arg("--codex-dir")
            .arg(codex.path())
            .arg("--include-subagents");
        command
    };

    command()
        .arg("worker-only")
        .assert()
        .success()
        .stdout(predicates::str::contains("c0de0015"))
        .stdout(predicates::str::contains("[subagent: fixture-worker]"));
    command()
        .arg("sessions")
        .arg("--all")
        .assert()
        .success()
        .stdout(predicates::str::contains("c0de0015"))
        .stdout(predicates::str::contains("[subagent: fixture-worker]"));
    command()
        .arg("show")
        .arg("c0de0015")
        .assert()
        .success()
        .stdout(predicates::str::contains("worker-only marker"));
}

#[test]
fn codex_home_override_and_missing_stores_are_silent() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    plant_codex_session(
        codex.path(),
        "c0de0006-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"environment marker"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CODEX_HOME", codex.path())
        .arg("--claude-dir")
        .arg(workdir.path().join("missing-claude"))
        .arg("environment")
        .assert()
        .success()
        .stdout(predicates::str::contains("c0de0006"));
}

#[test]
fn codex_show_and_session_search_use_the_transparent_id_handoff() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let id = "c0de0010-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        id,
        workdir.path(),
        "user",
        &[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"inspect the build"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"check the compiler output"}],"encrypted_content":"opaque"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","call_id":"call-1","name":"exec_command","input":"{\"cmd\":\"cargo check\"}"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call-1","output":"checking complete"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"handoff reply marker"}]}}"#,
        ],
    );

    let command = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(workdir.path().join("missing-claude"))
            .arg("--codex-dir")
            .arg(codex.path());
        command
    };

    command()
        .arg("show")
        .arg("c0de0010")
        .assert()
        .success()
        .stdout(predicates::str::contains("inspect the build"))
        .stdout(predicates::str::contains("→ exec_command cargo check"))
        .stdout(predicates::str::contains("← checking complete"))
        .stdout(predicates::str::contains("codex\n"))
        .stdout(predicates::str::contains("claude\n").not())
        .stdout(predicates::str::contains("thinking: 1 line hidden"))
        .stdout(predicates::str::contains("check the compiler output").not());

    command()
        .arg("show")
        .arg("c0de0010")
        .arg("--thinking")
        .assert()
        .success()
        .stdout(predicates::str::contains("check the compiler output"));

    command()
        .arg("handoff reply")
        .arg("--session")
        .arg("c0de0010")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "[2] assistant: handoff reply marker",
        ));

    command()
        .arg("show")
        .arg("c0de0010")
        .arg("--around")
        .arg("2")
        .arg("--context")
        .arg("0")
        .assert()
        .success()
        .stdout(predicates::str::contains("handoff reply marker"))
        .stdout(predicates::str::contains("inspect the build").not());
}

#[test]
fn codex_search_content_flags_match_claude_semantics() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    plant_codex_session(
        codex.path(),
        "c0de0011-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[
            r#"{"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"reasoning-only-needle"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"reasoning","content":[{"type":"reasoning_text","text":"content-reasoning-needle"}]}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","call_id":"call-1","name":"exec_command","input":"{\"cmd\":\"tool-call-needle\"}"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call-1","output":"tool-output-needle"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"finished"}]}}"#,
        ],
    );
    let command = |query: &str| {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(workdir.path().join("missing-claude"))
            .arg("--codex-dir")
            .arg(codex.path())
            .arg(query);
        command
    };

    command("needle")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
    command("reasoning-only")
        .arg("--thinking")
        .assert()
        .success()
        .stdout(predicates::str::contains("reasoning-only-needle"));
    command("content-reasoning")
        .arg("--thinking")
        .assert()
        .success()
        .stdout(predicates::str::contains("[1] thinking:"))
        .stdout(predicates::str::contains("content-reasoning-needle"));
    command("show")
        .arg("c0de0011")
        .arg("--around")
        .arg("1")
        .arg("--context")
        .arg("0")
        .arg("--thinking")
        .assert()
        .success()
        .stdout(predicates::str::contains("content-reasoning-needle"));
    command("tool-output")
        .arg("--tools")
        .assert()
        .success()
        .stdout(predicates::str::contains("tool-output-needle"));
    command("tool-call")
        .arg("--all-content")
        .assert()
        .success()
        .stdout(predicates::str::contains("tool-call-needle"));
}

#[test]
fn sessions_lists_codex_titles_recency_and_excludes_subagents() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let titled = "c0de0012-0000-0000-0000-000000000000";
    let untitled = "c0de0013-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        titled,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"new"}]}}"#,
        ],
    );
    let old_path = plant_codex_session(
        codex.path(),
        untitled,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-01-01T10:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"old"}]}}"#,
        ],
    );
    let old_text = fs::read_to_string(&old_path).unwrap().replacen(
        "2026-08-28T10:00:00.000Z",
        "2026-01-01T09:00:00.000Z",
        1,
    );
    fs::write(old_path, old_text).unwrap();
    plant_codex_session(
        codex.path(),
        "c0de0014-0000-0000-0000-000000000000",
        workdir.path(),
        "subagent",
        &[
            r#"{"timestamp":"2026-08-28T11:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"worker"}]}}"#,
        ],
    );
    fs::write(
        codex.path().join("session_index.jsonl"),
        serde_json::json!({"id": titled, "thread_name": "Indexed Codex title", "updated_at": "2026-08-28T10:00:00Z"}).to_string(),
    )
    .unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(workdir.path().join("missing-claude"))
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("sessions")
        .arg("--all")
        .arg("--since")
        .arg("2026-08-01")
        .assert()
        .success()
        .stdout(predicates::str::contains("codex · c0de0012 · "))
        .stdout(predicates::str::contains("Indexed Codex title"))
        .stdout(predicates::str::contains("c0de0013").not())
        .stdout(predicates::str::contains("c0de0014").not());

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(workdir.path().join("missing-claude"))
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("sessions")
        .arg("--all")
        .assert()
        .success()
        .stdout(predicates::str::contains("c0de0013"))
        .stdout(predicates::str::contains("(untitled)"));

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(workdir.path().join("missing-claude"))
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("Indexed Codex title")
        .assert()
        .success()
        .stdout(predicates::str::contains("codex · c0de0012"));
}

#[test]
fn projects_merge_harnesses_and_cwdless_codex_sessions_stay_unscoped() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let encoded = agsearch::encode_project_dir(&workdir.path().to_string_lossy());
    plant_project(
        claude.path(),
        &encoded.to_uppercase(),
        &format!(
            r#"{{"type":"user","message":{{"role":"user","content":"claude project marker"}},"timestamp":"2026-01-01T10:00:00.000Z","cwd":{}}}"#,
            serde_json::to_string(&workdir.path().to_string_lossy()).unwrap()
        ),
    );
    plant_codex_session(
        codex.path(),
        "c0de0016-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T12:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex project marker"}]}}"#,
        ],
    );
    let cwdless_dir = codex.path().join("sessions/2026/08/28");
    let cwdless_path = cwdless_dir.join("rollout-cwdless.jsonl");
    fs::write(
        &cwdless_path,
        [
            r#"{"timestamp":"2026-08-28T13:00:00.000Z","type":"session_meta","payload":{"id":"c0de0017-0000-0000-0000-000000000000","thread_source":"user"}}"#,
            r#"{"timestamp":"2026-08-28T13:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"cwdless marker"}]}}"#,
        ]
        .join("\n"),
    )
    .unwrap();
    let command = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--codex-dir")
            .arg(codex.path());
        command
    };

    command()
        .arg("projects")
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "{} · 2 sessions · 2026-08-28",
            workdir.path().display()
        )))
        .stdout(predicates::str::contains(" · 1 session · 2026-08-28").not());
    command()
        .arg("codex project")
        .assert()
        .success()
        .stdout(predicates::str::contains("c0de0016"));
    command()
        .arg("codex project")
        .arg("--project")
        .arg("tmp")
        .assert()
        .success()
        .stdout(predicates::str::contains("c0de0016"));
    command()
        .arg("cwdless")
        .arg("--all")
        .assert()
        .success()
        .stdout(predicates::str::contains("c0de0017"));
    command()
        .arg("show")
        .arg("c0de0017")
        .assert()
        .success()
        .stdout(predicates::str::contains("cwdless marker"));
}

#[test]
fn codex_failed_and_stats_infer_failures_from_tool_outputs() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    plant_codex_session(
        codex.path(),
        "c0de0018-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","call_id":"ok","name":"exec","input":"{\"cmd\":\"cargo check\"}"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"ok","output":"{\"exit_code\":0,\"output\":\"Finished\"}"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","call_id":"bad","name":"exec","input":"{\"cmd\":\"cargo test\"}"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"bad","output":"{\"exit_code\":101,\"output\":\"error[E0433]: failed to resolve\"}"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"failure reported"}]}}"#,
        ],
    );
    let command = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(workdir.path().join("missing-claude"))
            .arg("--codex-dir")
            .arg(codex.path());
        command
    };

    command()
        .arg("--failed")
        .assert()
        .success()
        .stdout(predicates::str::contains("codex · c0de0018"))
        .stdout(predicates::str::contains("cargo test"))
        .stdout(predicates::str::contains("exit 101"))
        .stdout(predicates::str::contains("error[E0433]: failed to resolve"))
        .stdout(predicates::str::contains("cargo check").not());
    command()
        .arg("--stats")
        .assert()
        .success()
        .stdout(predicates::str::contains("1  ✗ exec"))
        .stdout(predicates::str::contains("error[EN]: failed to resolve"));
}

#[test]
fn all_flag_searches_projects_other_than_the_current_one() {
    let workdir = tempdir().unwrap(); // cwd has no Project of its own
    let store = tempdir().unwrap();
    plant_project(
        store.path(),
        "E--projects-other",
        r#"{"type":"user","message":{"role":"user","content":"run the diesel migration"}}"#,
    );

    agsearch_command()
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
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

    agsearch_command()
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"anything"}}"#,
    );

    cmd.arg("--regex")
        .arg("foo(bar")
        .assert()
        .failure()
        .stderr(predicates::str::contains("agsearch:"))
        .stderr(predicates::str::contains("regex"));
}

#[test]
fn thinking_flag_includes_thinking_blocks() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"zebra one"}}"#,
        r#"{"type":"user","message":{"role":"user","content":"zebra two"}}"#,
        r#"{"type":"user","message":{"role":"user","content":"zebra three"}}"#,
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("-m")
        .arg("1")
        .arg("zebra")
        .assert()
        .success()
        .stdout(predicates::str::contains("+2 more"));
}

#[test]
fn files_flag_prints_only_the_matching_path() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
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
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"nothing relevant here"}}"#,
    );

    cmd.arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches"));
}

#[test]
fn search_output_carries_the_session_id_and_turn_handoff() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let projects = store.path().join("projects");
    let encoded = agsearch::encode_project_dir(&workdir.path().to_string_lossy());
    let project_dir = projects.join(encoded);
    fs::create_dir_all(&project_dir).unwrap();
    // A two-Message Session whose id has a recognisable short prefix.
    fs::write(
        project_dir.join("abcd1234-feed-feed-feed-feedfeedfeed.jsonl"),
        [
            r#"{"type":"user","message":{"role":"user","content":"first tokio question"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"a reply about tokio"}]}}"#,
        ]
        .join("\n"),
    )
    .unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("tokio")
        .assert()
        .success()
        // Header leads with the short session-id; matches carry their turns.
        .stdout(predicates::str::contains("abcd1234"))
        .stdout(predicates::str::contains("[1] user:"))
        .stdout(predicates::str::contains("[2] assistant:"));
}

#[test]
fn failed_lists_failures_by_structure_with_the_handoff_shape() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"Exit code 101\nerror[E0433]: failed to resolve"}]}}"#,
        ]
        .join("\n"),
    );

    // No query: lists all Failures in scope, joined to the failing tool.
    cmd.arg("--failed")
        .assert()
        .success()
        .stdout(predicates::str::contains("✗ Bash"))
        .stdout(predicates::str::contains("cargo test"))
        .stdout(predicates::str::contains(
            "exit 101 · error[E0433]: failed to resolve",
        ));
}

#[test]
fn stats_aggregates_failures_into_a_counts_table_by_signature() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"PowerShell","input":{"command":"cargo test x"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","is_error":true,"content":"Exit code 101\nerror[E0433]: cannot find type"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"b","name":"PowerShell","input":{"command":"cargo test y"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"b","is_error":true,"content":"Exit code 101\nerror[E0609]: cannot find type"}]}}"#,
        ]
        .join("\n"),
    );

    // Two compile errors of the same *shape* (only the masked code differs)
    // collapse into one structural group of 2 — not a hard-coded `error[` label.
    cmd.arg("--stats")
        .assert()
        .success()
        .stdout(predicates::str::contains("✗ PowerShell"))
        .stdout(predicates::str::contains("error[EN]: cannot find type"))
        .stdout(predicates::str::contains("2"));
}

#[test]
fn failed_query_filters_and_full_shows_everything() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"Exit code 101\nmid line\nerror[E0433]: failed to resolve"}]}}"#,
        ]
        .join("\n"),
    );

    cmd.arg("cargo")
        .arg("--failed")
        .arg("--full")
        .assert()
        .success()
        .stdout(predicates::str::contains("mid line")); // --full shows non-salient lines
}

#[test]
fn since_excludes_sessions_older_than_an_absolute_date() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // Both Sessions in the cwd's own Project, so the default scope covers them.
    let project = store
        .path()
        .join("projects")
        .join(agsearch::encode_project_dir(
            &workdir.path().to_string_lossy(),
        ));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("newish.jsonl"),
        r#"{"type":"user","message":{"role":"user","content":"recent tokio talk"},"timestamp":"2026-06-01T10:00:00.000Z"}"#,
    )
    .unwrap();
    fs::write(
        project.join("oldie.jsonl"),
        r#"{"type":"user","message":{"role":"user","content":"ancient tokio talk"},"timestamp":"2026-01-01T10:00:00.000Z"}"#,
    )
    .unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--since")
        .arg("2026-05-01")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("recent tokio talk"))
        .stdout(predicates::str::contains("ancient tokio talk").not());
}

#[test]
fn since_rejects_an_unparseable_value() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-06-01T10:00:00.000Z"}"#,
    );

    cmd.arg("--since")
        .arg("yesterday")
        .arg("x")
        .assert()
        .failure()
        .stderr(predicates::str::contains("could not parse --since"));
}

#[test]
fn session_scope_searches_only_the_named_session() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // Two Sessions in the same Project; the Query matches both.
    plant_session(
        store.path(),
        "E--projects-demo",
        "aaaa1111-0000-0000-0000-000000000000",
        r#"{"type":"user","message":{"role":"user","content":"tokio in session A"}}"#,
    );
    plant_session(
        store.path(),
        "E--projects-demo",
        "bbbb2222-0000-0000-0000-000000000000",
        r#"{"type":"user","message":{"role":"user","content":"tokio in session B"}}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--session")
        .arg("aaaa1111")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("tokio in session A"))
        .stdout(predicates::str::contains("tokio in session B").not());
}

#[test]
fn session_scope_errors_on_an_ambiguous_prefix() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let line = r#"{"type":"user","message":{"role":"user","content":"x"}}"#;
    plant_session(store.path(), "E--projects-demo", "dupe1111-aaaa", line);
    plant_session(store.path(), "E--projects-demo", "dupe2222-bbbb", line);

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--session")
        .arg("dupe")
        .arg("x")
        .assert()
        .failure()
        .stderr(predicates::str::contains("ambiguous"));
}

#[test]
fn session_scope_requires_a_query() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_session(
        store.path(),
        "E--projects-demo",
        "aaaa1111-0000-0000-0000-000000000000",
        r#"{"type":"user","message":{"role":"user","content":"x"}}"#,
    );

    // --session without a Query is not "dump the session" — that is show's job.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--session")
        .arg("aaaa1111")
        .assert()
        .failure()
        .stderr(predicates::str::contains("query is required"));
}

#[test]
fn an_empty_query_errors_like_a_missing_one() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"anything"}}"#,
    );

    // "" is a substring of everything — a shell variable that expands to empty
    // must fail fast, not dump the whole Project.
    cmd.arg("")
        .assert()
        .failure()
        .stderr(predicates::str::contains("query is required"));
}

#[test]
fn a_one_character_query_still_searches() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"zebra"}}"#,
    );

    // Only "" is rejected — the shortest real query is untouched.
    cmd.arg("z")
        .assert()
        .success()
        .stdout(predicates::str::contains("zebra"));
}

#[test]
fn a_whitespace_only_query_still_searches() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"one two"}}"#,
    );

    // A user who quotes a space may mean it; only the truly empty string is
    // rejected.
    cmd.arg(" ")
        .assert()
        .success()
        .stdout(predicates::str::contains("one two"));
}

#[test]
fn an_empty_failed_filter_means_no_filter() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"Exit code 101\nerror: boom"}]}}"#,
        ]
        .join("\n"),
    );

    // The --failed query is an optional *filter*; empty filter = unfiltered.
    cmd.arg("")
        .arg("--failed")
        .assert()
        .success()
        .stdout(predicates::str::contains("✗ Bash"));
}

#[test]
fn session_scope_conflicts_with_all() {
    agsearch_command()
        .arg("--session")
        .arg("abc")
        .arg("--all")
        .arg("x")
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn explicit_search_verb_behaves_like_a_bare_query() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"how to use tokio select"}}"#,
    );

    cmd.arg("search")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("how to use tokio select"));
}

/// Plant a Session with a known id under a named Project and return its full id.
fn plant_session(store_root: &std::path::Path, project: &str, id: &str, lines: &str) -> String {
    let dir = store_root.join("projects").join(project);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{id}.jsonl")), lines).unwrap();
    id.to_string()
}

#[test]
fn show_renders_a_transcript_resolved_from_a_session_id_prefix() {
    let store = tempdir().unwrap();
    let id = plant_session(
        store.path(),
        "E--projects-demo",
        "4c28878f-c921-4892-8d26-78df5801f301",
        &[
            r#"{"type":"user","message":{"role":"user","content":"fix the failing build"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me run the tests."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"error[E0433]: failed to resolve"}]}}"#,
        ]
        .join("\n"),
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg(&id[..8]) // git-style prefix
        .assert()
        .success()
        .stdout(predicates::str::contains("you"))
        .stdout(predicates::str::contains("fix the failing build"))
        .stdout(predicates::str::contains("claude"))
        .stdout(predicates::str::contains("→ Bash cargo test"))
        .stdout(predicates::str::contains("✗ Bash FAILED"))
        .stdout(predicates::str::contains("E0433"));
}

#[test]
fn show_errors_cleanly_on_an_ambiguous_prefix() {
    let store = tempdir().unwrap();
    let line = r#"{"type":"user","message":{"role":"user","content":"x"}}"#;
    plant_session(store.path(), "E--projects-demo", "abc111-aaaa", line);
    plant_session(store.path(), "E--projects-demo", "abc222-bbbb", line);

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg("abc")
        .assert()
        .failure()
        .stderr(predicates::str::contains("ambiguous"))
        .stderr(predicates::str::contains("abc111-aaaa"))
        .stderr(predicates::str::contains("abc222-bbbb"));
}

#[test]
fn show_errors_cleanly_when_no_session_matches() {
    let store = tempdir().unwrap();
    plant_session(
        store.path(),
        "E--projects-demo",
        "abc111-aaaa",
        r#"{"type":"user","message":{"role":"user","content":"x"}}"#,
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg("zzz")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no session matches"));
}

#[test]
fn show_dash_reads_a_session_path_from_stdin() {
    let store = tempdir().unwrap();
    let dir = store.path().join("projects").join("E--projects-demo");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("piped.jsonl");
    fs::write(
        &path,
        r#"{"type":"user","message":{"role":"user","content":"piped in from a path"}}"#,
    )
    .unwrap();

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg("-")
        .write_stdin(format!("{}\n", path.display()))
        .assert()
        .success()
        .stdout(predicates::str::contains("piped in from a path"));
}

#[test]
fn show_dash_reads_a_relative_path_outside_configured_stores() {
    let workdir = tempdir().unwrap();
    fs::write(
        workdir.path().join("relative.jsonl"),
        r#"{"type":"user","message":{"role":"user","content":"relative path session"}}"#,
    )
    .unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(workdir.path().join("missing-claude"))
        .arg("show")
        .arg("-")
        .write_stdin("relative.jsonl\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("relative path session"));
}

#[test]
fn show_around_windows_the_transcript_on_a_turn() {
    let store = tempdir().unwrap();
    // Six user prompts → turns 1..=6, each with a unique marker.
    let lines: String = (1..=6)
        .map(|i| {
            format!(
                r#"{{"type":"user","message":{{"role":"user","content":"prompt number {i}"}}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let id = plant_session(
        store.path(),
        "E--projects-demo",
        "feed0001-0000-0000-0000-000000000000",
        &lines,
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg(&id[..8])
        .arg("--around")
        .arg("3")
        .arg("--context")
        .arg("1")
        .assert()
        .success()
        // Window is turns 2..=4; turns 1 and 5,6 are hidden with indicators.
        .stdout(predicates::str::contains("prompt number 3"))
        .stdout(predicates::str::contains("prompt number 2"))
        .stdout(predicates::str::contains("prompt number 4"))
        .stdout(predicates::str::contains("prompt number 1").not())
        .stdout(predicates::str::contains("prompt number 5").not())
        .stdout(predicates::str::contains("1 earlier turn hidden"))
        .stdout(predicates::str::contains("2 later turns hidden"));
}

#[test]
fn bare_show_is_unaffected_by_the_default_context() {
    let store = tempdir().unwrap();
    let lines: String = (1..=6)
        .map(|i| {
            format!(
                r#"{{"type":"user","message":{{"role":"user","content":"prompt number {i}"}}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let id = plant_session(
        store.path(),
        "E--projects-demo",
        "feed0002-0000-0000-0000-000000000000",
        &lines,
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg(&id[..8])
        .assert()
        .success()
        // No --around: the whole Transcript, no hidden-turn indicators.
        .stdout(predicates::str::contains("prompt number 1"))
        .stdout(predicates::str::contains("prompt number 6"))
        .stdout(predicates::str::contains("hidden").not());
}

#[test]
fn show_collapses_thinking_by_default_and_expands_with_the_flag() {
    let store = tempdir().unwrap();
    let id = plant_session(
        store.path(),
        "E--projects-demo",
        "deadbeef-0000-0000-0000-000000000000",
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a secret rumination"},{"type":"text","text":"here is my reply"}]}}"#,
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg(&id[..8])
        .assert()
        .success()
        .stdout(predicates::str::contains("[thinking:"))
        .stdout(predicates::str::contains("a secret rumination").not());

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg(&id[..8])
        .arg("--thinking")
        .assert()
        .success()
        .stdout(predicates::str::contains("a secret rumination"));
}

// --- sessions: list Sessions in a scope (issue 16, ADR 0004) -------------

#[test]
fn sessions_lists_the_current_projects_sessions_newest_first() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let project = store
        .path()
        .join("projects")
        .join(agsearch::encode_project_dir(
            &workdir.path().to_string_lossy(),
        ));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("aaaa1111-0000-0000-0000-000000000000.jsonl"),
        [
            r#"{"type":"ai-title","aiTitle":"Older conversation"}"#,
            r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-01-01T10:00:00.000Z"}"#,
        ]
        .join("\n"),
    )
    .unwrap();
    fs::write(
        project.join("bbbb2222-1111-1111-1111-111111111111.jsonl"),
        [
            r#"{"type":"ai-title","aiTitle":"Newer conversation"}"#,
            r#"{"type":"user","message":{"role":"user","content":"hey"},"timestamp":"2026-06-01T10:00:00.000Z"}"#,
        ]
        .join("\n"),
    )
    .unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("sessions")
        .assert()
        .success()
        // Both Sessions appear as header lines with their short-ids and Titles.
        .stdout(predicates::str::contains("bbbb2222 · "))
        .stdout(predicates::str::contains("Newer conversation"))
        .stdout(predicates::str::contains("aaaa1111 · "))
        .stdout(predicates::str::contains("Older conversation"))
        // Newest first: the newer short-id precedes the older one.
        .stdout(predicates::function::function(|out: &str| {
            out.find("bbbb2222").unwrap() < out.find("aaaa1111").unwrap()
        }));
}

#[test]
fn sessions_renders_untitled_and_omits_no_content_filter() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let project = store
        .path()
        .join("projects")
        .join(agsearch::encode_project_dir(
            &workdir.path().to_string_lossy(),
        ));
    fs::create_dir_all(&project).unwrap();
    // A Session with no ai-title Record and no Message at all (only a noise
    // Record). search would never surface it; `sessions` must still list it.
    fs::write(
        project.join("dddd4444-0000-0000-0000-000000000000.jsonl"),
        r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-04-01T10:00:00.000Z"}"#,
    )
    .unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains("dddd4444 · "))
        .stdout(predicates::str::contains("(untitled)"));
}

#[test]
fn sessions_reports_cleanly_when_the_scope_is_empty() {
    let workdir = tempdir().unwrap(); // cwd has no Project in the Store
    let store = tempdir().unwrap();
    fs::create_dir_all(store.path().join("projects")).unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("sessions")
        .assert()
        .success() // "nothing here" is not an error (mirrors search's no-match)
        .stdout(predicates::str::contains("No sessions."));
}

#[test]
fn sessions_all_widens_scope_and_since_excludes_older() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // Two other Projects, each with one Session of a different age.
    plant_project(
        store.path(),
        "E--projects-recent",
        r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-06-01T10:00:00.000Z"}"#,
    );
    plant_project(
        store.path(),
        "E--projects-ancient",
        r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-01-01T10:00:00.000Z"}"#,
    );

    // Without --all, the cwd's (empty) Project yields nothing.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains("No sessions."));

    // --all sees both; --since 2026-05-01 keeps only the recent one.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("sessions")
        .arg("--all")
        .arg("--since")
        .arg("2026-05-01")
        .assert()
        .success()
        .stdout(predicates::str::contains("E--projects-recent"))
        .stdout(predicates::str::contains("E--projects-ancient").not());
}

#[test]
fn sessions_files_prints_paths_for_piping_into_show() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let project = store
        .path()
        .join("projects")
        .join(agsearch::encode_project_dir(
            &workdir.path().to_string_lossy(),
        ));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("eeee5555-0000-0000-0000-000000000000.jsonl"),
        r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-06-01T10:00:00.000Z"}"#,
    )
    .unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("sessions")
        .arg("-l")
        .assert()
        .success()
        // Only the path, not a header line (no " · " separators).
        .stdout(predicates::str::contains(
            "eeee5555-0000-0000-0000-000000000000.jsonl",
        ))
        .stdout(predicates::str::contains(" · ").not());
}

#[test]
fn sessions_rejects_search_only_flags() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    fs::create_dir_all(store.path().join("projects")).unwrap();

    for flag in ["--failed", "--thinking", "--tools", "--stats"] {
        agsearch_command()
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(store.path())
            .arg("sessions")
            .arg(flag)
            .assert()
            .failure(); // clap rejects an argument `sessions` does not define
    }
}

// --- projects: list Projects in the Store (issue 17, ADR 0005) -----------

#[test]
fn projects_lists_projects_by_real_cwd_with_counts_newest_first() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // A recent Project with two Sessions, and an older Project with one.
    plant_project(
        store.path(),
        "E--projects-recent",
        r#"{"type":"user","message":{"role":"user","content":"a"},"timestamp":"2026-06-01T10:00:00.000Z","cwd":"E:\\projects\\recent"}"#,
    );
    // Second Session in the recent Project (distinct file).
    fs::write(
        store.path().join("projects").join("E--projects-recent").join("second.jsonl"),
        r#"{"type":"user","message":{"role":"user","content":"b"},"timestamp":"2026-06-02T10:00:00.000Z","cwd":"E:\\projects\\recent"}"#,
    )
    .unwrap();
    plant_project(
        store.path(),
        "E--projects-ancient",
        r#"{"type":"user","message":{"role":"user","content":"c"},"timestamp":"2026-01-01T10:00:00.000Z","cwd":"E:\\projects\\ancient"}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("projects")
        .assert()
        .success()
        // Real cwd as the name, with a Session count.
        .stdout(predicates::str::contains(
            "E:\\projects\\recent · 2 sessions · 2026-06-02",
        ))
        .stdout(predicates::str::contains(
            "E:\\projects\\ancient · 1 session · 2026-01-01",
        ))
        // Newest-touched first.
        .stdout(predicates::function::function(|out: &str| {
            out.find("recent").unwrap() < out.find("ancient").unwrap()
        }));
}

#[test]
fn projects_since_and_project_filter_compose() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_project(
        store.path(),
        "E--projects-recent",
        r#"{"type":"user","message":{"role":"user","content":"a"},"timestamp":"2026-06-01T10:00:00.000Z","cwd":"E:\\projects\\recent"}"#,
    );
    plant_project(
        store.path(),
        "E--projects-ancient",
        r#"{"type":"user","message":{"role":"user","content":"c"},"timestamp":"2026-01-01T10:00:00.000Z","cwd":"E:\\projects\\ancient"}"#,
    );

    // --since drops the ancient Project.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("projects")
        .arg("--since")
        .arg("2026-05-01")
        .assert()
        .success()
        .stdout(predicates::str::contains("recent"))
        .stdout(predicates::str::contains("ancient").not());

    // --project filters by directory-name substring.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("projects")
        .arg("--project")
        .arg("ancient")
        .assert()
        .success()
        .stdout(predicates::str::contains("ancient"))
        .stdout(predicates::str::contains("recent").not());
}

#[test]
fn projects_reports_cleanly_when_the_store_is_empty() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    fs::create_dir_all(store.path().join("projects")).unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("projects")
        .assert()
        .success()
        .stdout(predicates::str::contains("No projects."));
}

#[test]
fn projects_rejects_all_and_files_flags() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    fs::create_dir_all(store.path().join("projects")).unwrap();

    for flag in ["--all", "-l"] {
        agsearch_command()
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(store.path())
            .arg("projects")
            .arg(flag)
            .assert()
            .failure(); // clap rejects an argument `projects` does not define
    }
}

#[test]
fn harness_flag_can_select_the_only_functional_claude_adapter() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"adapter seam"}}"#,
    );

    cmd.arg("adapter")
        .arg("--harness")
        .arg("claude")
        .assert()
        .success()
        .stdout(predicates::str::contains("adapter seam"));
}

#[test]
fn codex_harness_selection_ignores_the_claude_store() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"claude only"}}"#,
    );

    cmd.arg("anything")
        .arg("--harness")
        .arg("codex")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
}

#[test]
fn current_fails_without_guessing_the_newest_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "zzzzzzzz-0000-0000-0000-000000000000",
        r#"{"type":"user","message":{"role":"user","content":"newest prompt"},"timestamp":"2026-08-01T10:00:00.000Z","cwd":"E:\\projects\\demo"}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("current")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no current Session"))
        .stdout(predicates::str::contains("zzzzzzzz").not());
}

#[test]
fn current_fails_when_a_claude_workers_parent_is_absent_from_the_store() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let parent_id = "11111111-aaaa-bbbb-cccc-ddddeeee0030";
    let worker_id = "11111111-aaaa-bbbb-cccc-ddddeeee0031";
    plant_claude_subagent(
        claude.path(),
        workdir.path(),
        parent_id,
        worker_id,
        r#"{"type":"user","message":{"role":"user","content":"orphan worker"}}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("current")
        .assert()
        .failure()
        .stderr(predicates::str::contains(parent_id))
        .stderr(predicates::str::contains("was not found"));
}

#[test]
fn current_prints_claude_session_metadata() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0001";
    let path = plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        &[
            r#"{"type":"ai-title","aiTitle":"Borrow checker chat"}"#,
            r#"{"type":"user","message":{"role":"user","content":"how to borrow"},"timestamp":"2026-08-01T10:00:00.000Z","cwd":"E:\\projects\\demo","gitBranch":"main"}"#,
        ]
        .join("\n"),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", session_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("current")
        .assert()
        .success()
        .stdout(predicates::str::contains("Harness: claude"))
        .stdout(predicates::str::contains(format!("Session: {session_id}")))
        .stdout(predicates::str::contains("Project: E:\\projects\\demo"))
        .stdout(predicates::str::contains("Title: Borrow checker chat"))
        .stdout(predicates::str::contains(format!(
            "Path: {}",
            path.display()
        )))
        .stdout(predicates::str::contains("Caller: top-level"));
}

#[test]
fn current_resolves_a_claude_worker_to_the_top_level_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let parent_id = "11111111-aaaa-bbbb-cccc-ddddeeee0020";
    let worker_id = "11111111-aaaa-bbbb-cccc-ddddeeee0021";
    let parent_path = plant_claude_session(
        claude.path(),
        workdir.path(),
        parent_id,
        &[
            r#"{"type":"ai-title","aiTitle":"Parent conversation"}"#,
            r#"{"type":"user","message":{"role":"user","content":"parent prompt"},"cwd":"E:\\projects\\demo"}"#,
        ]
        .join("\n"),
    );
    plant_claude_subagent(
        claude.path(),
        workdir.path(),
        parent_id,
        worker_id,
        r#"{"type":"user","message":{"role":"user","content":"worker prompt"},"cwd":"E:\\projects\\demo"}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("current")
        .assert()
        .success()
        .stdout(predicates::str::contains("Harness: claude"))
        .stdout(predicates::str::contains(format!("Session: {parent_id}")))
        .stdout(predicates::str::contains("Title: Parent conversation"))
        .stdout(predicates::str::contains(format!(
            "Path: {}",
            parent_path.display()
        )))
        .stdout(predicates::str::contains("Caller: worker"));
}

#[test]
fn current_id_only_prints_the_full_top_level_session_id() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0002";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        r#"{"type":"user","message":{"role":"user","content":"id only"}}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", session_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("current")
        .arg("--id-only")
        .assert()
        .success()
        .stdout(predicates::str::is_match(format!("^{session_id}\n$")).unwrap());
}

#[test]
fn current_path_prints_only_the_source_session_path() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0003";
    let path = plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        r#"{"type":"user","message":{"role":"user","content":"path only"}}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", session_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("current")
        .arg("--path")
        .assert()
        .success()
        .stdout(format!("{}\n", path.display()));
}

#[test]
fn current_prints_codex_session_metadata_at_the_top_level() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let session_id = "c0de1001-0000-0000-0000-000000000000";
    let path = plant_codex_session(
        codex.path(),
        session_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex current"}]}}"#,
        ],
    );
    plant_codex_title(codex.path(), session_id, "Codex current chat");

    agsearch_command()
        .current_dir(workdir.path())
        .env("CODEX_SESSION_ID", session_id)
        .env("CODEX_THREAD_ID", session_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("current")
        .assert()
        .success()
        .stdout(predicates::str::contains("Harness: codex"))
        .stdout(predicates::str::contains(format!("Session: {session_id}")))
        .stdout(predicates::str::contains("Title: Codex current chat"))
        .stdout(predicates::str::contains(format!(
            "Path: {}",
            path.display()
        )))
        .stdout(predicates::str::contains("Caller: top-level"));
}

#[test]
fn current_resolves_a_codex_worker_to_the_top_level_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let parent_id = "c0de2001-0000-0000-0000-000000000000";
    let worker_id = "c0de2002-0000-0000-0000-000000000000";
    let parent_path = plant_codex_session(
        codex.path(),
        parent_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"parent prompt"}]}}"#,
        ],
    );
    plant_codex_subagent(
        codex.path(),
        worker_id,
        workdir.path(),
        parent_id,
        &[
            r#"{"timestamp":"2026-08-28T10:02:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"worker prompt"}]}}"#,
        ],
    );
    plant_codex_title(codex.path(), parent_id, "Parent conversation");

    let command = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .env("CODEX_SESSION_ID", parent_id)
            .env("CODEX_THREAD_ID", worker_id)
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--codex-dir")
            .arg(codex.path())
            .arg("current");
        command
    };

    command()
        .assert()
        .success()
        .stdout(predicates::str::contains("Harness: codex"))
        .stdout(predicates::str::contains(format!("Session: {parent_id}")))
        .stdout(predicates::str::contains("Title: Parent conversation"))
        .stdout(predicates::str::contains(format!(
            "Path: {}",
            parent_path.display()
        )))
        .stdout(predicates::str::contains("Caller: worker"));
    command()
        .arg("--id-only")
        .assert()
        .success()
        .stdout(format!("{parent_id}\n"));
}

#[test]
fn current_fails_when_the_identity_is_absent_from_the_store() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        r#"{"type":"user","message":{"role":"user","content":"other session"}}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env(
            "CLAUDE_CODE_SESSION_ID",
            "bbbbbbbb-1111-1111-1111-111111111111",
        )
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("current")
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "bbbbbbbb-1111-1111-1111-111111111111",
        ))
        .stderr(predicates::str::contains("was not found"));
}

#[test]
fn current_reports_ambiguity_when_both_harnesses_identify_a_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let claude_id = "11111111-aaaa-bbbb-cccc-ddddeeee0010";
    let codex_id = "c0de3001-0000-0000-0000-000000000000";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        claude_id,
        r#"{"type":"user","message":{"role":"user","content":"claude current"}}"#,
    );
    plant_codex_session(
        codex.path(),
        codex_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex current"}]}}"#,
        ],
    );

    let command = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .env("CLAUDE_CODE_SESSION_ID", claude_id)
            .env("CODEX_SESSION_ID", codex_id)
            .env("CODEX_THREAD_ID", codex_id)
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--codex-dir")
            .arg(codex.path())
            .arg("current");
        command
    };

    command()
        .assert()
        .failure()
        .stderr(predicates::str::contains("ambiguous"))
        .stderr(predicates::str::contains("--harness"));
    command()
        .arg("--harness")
        .arg("codex")
        .assert()
        .success()
        .stdout(predicates::str::contains("Harness: codex"))
        .stdout(predicates::str::contains(format!("Session: {codex_id}")));
    command()
        .arg("--harness")
        .arg("claude")
        .assert()
        .success()
        .stdout(predicates::str::contains("Harness: claude"))
        .stdout(predicates::str::contains(format!("Session: {claude_id}")));
}

#[test]
fn current_ignores_a_stale_identity_when_the_other_harness_resolves() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let codex_id = "c0de4001-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        codex_id,
        workdir.path(),
        "user",
        &[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"valid codex current"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env(
            "CLAUDE_CODE_SESSION_ID",
            "aaaaaaaa-1111-1111-1111-111111111111",
        )
        .env("CODEX_SESSION_ID", codex_id)
        .env("CODEX_THREAD_ID", codex_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("current")
        .assert()
        .success()
        .stdout(predicates::str::contains("Harness: codex"))
        .stdout(predicates::str::contains(format!("Session: {codex_id}")));
}

#[test]
fn current_does_not_resolve_an_identity_from_the_other_harness_store() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let shared_id = "c0de4002-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        shared_id,
        workdir.path(),
        "user",
        &[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex only"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", shared_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("current")
        .assert()
        .failure()
        .stderr(predicates::str::contains("configured claude Store"));
}

#[test]
fn include_subagents_does_not_expose_claude_workers() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let parent_id = "11111111-aaaa-bbbb-cccc-ddddeeee0040";
    let worker_id = "11111111-aaaa-bbbb-cccc-ddddeeee0041";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        parent_id,
        r#"{"type":"user","message":{"role":"user","content":"parent prompt"}}"#,
    );
    plant_claude_subagent(
        claude.path(),
        workdir.path(),
        parent_id,
        worker_id,
        r#"{"type":"user","message":{"role":"user","content":"claude worker private marker"}}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--include-subagents")
        .arg("claude worker private marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
}

/// Both Sessions carry this Query, so a result tells you which one was searched.
const CURRENT_MARKER: &str = "shared marker in the current session";
const EARLIER_MARKER: &str = "shared marker in earlier work";

/// Two Sessions in the current Project, one of which is the Current Session.
/// Returns the Current Session's id.
fn plant_current_and_earlier_session(
    workdir: &std::path::Path,
    claude: &std::path::Path,
) -> String {
    let current_id = "11111111-aaaa-bbbb-cccc-ddddeeee0100";
    plant_claude_session(
        claude,
        workdir,
        current_id,
        &format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{CURRENT_MARKER}"}},"timestamp":"2026-08-02T10:00:00.000Z"}}"#
        ),
    );
    plant_claude_session(
        claude,
        workdir,
        "11111111-aaaa-bbbb-cccc-ddddeeee0101",
        &format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{EARLIER_MARKER}"}},"timestamp":"2026-08-01T10:00:00.000Z"}}"#
        ),
    );
    current_id.to_string()
}

/// A command run as if the Harness had invoked it from `current_id`.
fn agsearch_as_current(
    workdir: &std::path::Path,
    claude: &std::path::Path,
    current_id: &str,
) -> Command {
    let mut command = agsearch_command();
    command
        .current_dir(workdir)
        .env("CLAUDE_CODE_SESSION_ID", current_id)
        .arg("--claude-dir")
        .arg(claude);
    command
}

#[test]
fn project_search_excludes_the_current_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("shared marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("shared marker in earlier work"))
        .stdout(predicates::str::contains("shared marker in the current session").not());
}

#[test]
fn whole_store_search_excludes_the_current_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--all")
        .arg("shared marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("shared marker in earlier work"))
        .stdout(predicates::str::contains("shared marker in the current session").not());
}

#[test]
fn search_excludes_current_family_workers_under_include_subagents() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let current_id = "c0de5001-0000-0000-0000-000000000000";
    let worker_id = "c0de5002-0000-0000-0000-000000000000";
    let other_id = "c0de5003-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        current_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"family marker from the parent"}]}}"#,
        ],
    );
    plant_codex_subagent(
        codex.path(),
        worker_id,
        workdir.path(),
        current_id,
        &[
            r#"{"timestamp":"2026-08-28T10:02:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"family marker from the worker"}]}}"#,
        ],
    );
    plant_codex_session(
        codex.path(),
        other_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-27T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"family marker from earlier work"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CODEX_SESSION_ID", current_id)
        .env("CODEX_THREAD_ID", current_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("--include-subagents")
        .arg("family marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("family marker from earlier work"))
        .stdout(predicates::str::contains("family marker from the parent").not())
        .stdout(predicates::str::contains("family marker from the worker").not());
}

/// Plant a current Session and an earlier Session, each with one failed tool
/// call, so failure analysis has one Failure inside the family and one outside.
fn plant_current_and_earlier_failures(
    workdir: &std::path::Path,
    claude: &std::path::Path,
) -> String {
    let current_id = "11111111-aaaa-bbbb-cccc-ddddeeee0110";
    plant_claude_session(
        claude,
        workdir,
        current_id,
        &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"CurrentTool","input":{"command":"cargo build"}}]},"timestamp":"2026-08-02T10:00:00.000Z"}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"error: current failure"}]}}"#,
        ]
        .join("\n"),
    );
    plant_claude_session(
        claude,
        workdir,
        "11111111-aaaa-bbbb-cccc-ddddeeee0111",
        &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"EarlierTool","input":{"command":"cargo test"}}]},"timestamp":"2026-08-01T10:00:00.000Z"}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"error: earlier failure"}]}}"#,
        ]
        .join("\n"),
    );
    current_id.to_string()
}

#[test]
fn failed_excludes_the_current_session_family() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_failures(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--failed")
        .assert()
        .success()
        .stdout(predicates::str::contains("EarlierTool"))
        .stdout(predicates::str::contains("CurrentTool").not());
}

#[test]
fn stats_excludes_the_current_session_family() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_failures(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--stats")
        .assert()
        .success()
        .stdout(predicates::str::contains("EarlierTool"))
        .stdout(predicates::str::contains("CurrentTool").not());
}

#[test]
fn include_current_restores_the_family_for_search() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--include-current")
        .arg("shared marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("shared marker in earlier work"))
        .stdout(predicates::str::contains(
            "shared marker in the current session",
        ));
}

#[test]
fn include_current_restores_the_family_for_failure_analysis() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_failures(workdir.path(), claude.path());

    let command = |mode: &str| {
        let mut command = agsearch_as_current(workdir.path(), claude.path(), &current_id);
        command.arg("--include-current").arg(mode);
        command
    };

    command("--failed")
        .assert()
        .success()
        .stdout(predicates::str::contains("EarlierTool"))
        .stdout(predicates::str::contains("CurrentTool"));
    command("--stats")
        .assert()
        .success()
        .stdout(predicates::str::contains("EarlierTool"))
        .stdout(predicates::str::contains("CurrentTool"));
}

#[test]
fn include_current_restores_family_workers_under_include_subagents() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let current_id = "c0de5101-0000-0000-0000-000000000000";
    let worker_id = "c0de5102-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        current_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"restored marker from the parent"}]}}"#,
        ],
    );
    plant_codex_subagent(
        codex.path(),
        worker_id,
        workdir.path(),
        current_id,
        &[
            r#"{"timestamp":"2026-08-28T10:02:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"restored marker from the worker"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CODEX_SESSION_ID", current_id)
        .env("CODEX_THREAD_ID", current_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("--include-subagents")
        .arg("--include-current")
        .arg("restored marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("restored marker from the parent"))
        .stdout(predicates::str::contains("restored marker from the worker"));
}

#[test]
fn an_explicit_session_selector_searches_the_current_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--session")
        .arg(&current_id)
        .arg("shared marker")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "shared marker in the current session",
        ));
}

#[test]
fn current_session_selector_searches_the_current_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--session")
        .arg("current")
        .arg("shared marker")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "shared marker in the current session",
        ))
        .stdout(predicates::str::contains("shared marker in earlier work").not());
}

#[test]
fn current_family_exclusion_is_qualified_by_harness() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let shared_id = "11111111-aaaa-bbbb-cccc-ddddeeee0130";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        shared_id,
        r#"{"type":"user","message":{"role":"user","content":"collision marker in current Claude work"},"timestamp":"2026-08-02T10:00:00.000Z"}"#,
    );
    plant_codex_session(
        codex.path(),
        shared_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-01T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"collision marker in unrelated Codex work"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", shared_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("collision marker")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "collision marker in unrelated Codex work",
        ))
        .stdout(predicates::str::contains("collision marker in current Claude work").not());
}

#[test]
fn sessions_still_lists_the_current_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains(&current_id[..8]));
}

#[test]
fn projects_still_counts_the_current_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("projects")
        .assert()
        .success()
        .stdout(predicates::str::contains("2 sessions"));
}

#[test]
fn analysis_outside_a_harness_keeps_every_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("shared marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("shared marker in earlier work"))
        .stdout(predicates::str::contains(
            "shared marker in the current session",
        ));
}

#[test]
fn search_excludes_the_calling_thread_when_its_parent_is_missing() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let absent_parent_id = "c0de5201-0000-0000-0000-000000000000";
    let worker_id = "c0de5202-0000-0000-0000-000000000000";
    plant_codex_subagent(
        codex.path(),
        worker_id,
        workdir.path(),
        absent_parent_id,
        &[
            r#"{"timestamp":"2026-08-28T10:02:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"orphan marker from the worker"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CODEX_SESSION_ID", absent_parent_id)
        .env("CODEX_THREAD_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("--include-subagents")
        .arg("orphan marker")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
}

#[test]
fn search_excludes_both_families_when_the_harnesses_disagree() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let claude_id = "11111111-aaaa-bbbb-cccc-ddddeeee0120";
    let codex_id = "c0de5301-0000-0000-0000-000000000000";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        claude_id,
        r#"{"type":"user","message":{"role":"user","content":"ambiguous marker in the claude session"},"timestamp":"2026-08-02T10:00:00.000Z"}"#,
    );
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "11111111-aaaa-bbbb-cccc-ddddeeee0121",
        r#"{"type":"user","message":{"role":"user","content":"ambiguous marker in earlier work"},"timestamp":"2026-08-01T10:00:00.000Z"}"#,
    );
    plant_codex_session(
        codex.path(),
        codex_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"ambiguous marker in the codex session"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", claude_id)
        .env("CODEX_SESSION_ID", codex_id)
        .env("CODEX_THREAD_ID", codex_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("ambiguous marker")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "ambiguous marker in earlier work",
        ))
        .stdout(predicates::str::contains("ambiguous marker in the claude session").not())
        .stdout(predicates::str::contains("ambiguous marker in the codex session").not());
}

// --- current-context selectors in Session commands (issue 18) ---

#[test]
fn show_current_renders_the_top_level_claude_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0200";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        r#"{"type":"user","message":{"role":"user","content":"top-level marker for show current"}}"#,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", session_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("show")
        .arg("current")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "top-level marker for show current",
        ));
}

#[test]
fn show_current_thread_renders_the_calling_claude_worker_without_subagents() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let parent_id = "11111111-aaaa-bbbb-cccc-ddddeeee0210";
    let worker_id = "11111111-aaaa-bbbb-cccc-ddddeeee0211";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        parent_id,
        r#"{"type":"user","message":{"role":"user","content":"parent marker for show selectors"}}"#,
    );
    plant_claude_subagent(
        claude.path(),
        workdir.path(),
        parent_id,
        worker_id,
        r#"{"type":"user","message":{"role":"user","content":"worker marker for show current-thread"}}"#,
    );

    // `current` selects the top-level Session even when invoked from a worker.
    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("show")
        .arg("current")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "parent marker for show selectors",
        ))
        .stdout(predicates::str::contains("worker marker for show current-thread").not());

    // `current-thread` selects the calling thread without --include-subagents.
    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("show")
        .arg("current-thread")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "worker marker for show current-thread",
        ))
        .stdout(predicates::str::contains("parent marker for show selectors").not());
}

#[test]
fn show_current_and_current_thread_select_the_same_codex_session_at_the_top_level() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let session_id = "c0de6001-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        session_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"top-level marker for codex show selectors"}]}}"#,
        ],
    );

    for selector in ["current", "current-thread"] {
        agsearch_command()
            .current_dir(workdir.path())
            .env("CODEX_SESSION_ID", session_id)
            .env("CODEX_THREAD_ID", session_id)
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--codex-dir")
            .arg(codex.path())
            .arg("show")
            .arg(selector)
            .assert()
            .success()
            .stdout(predicates::str::contains(
                "top-level marker for codex show selectors",
            ));
    }
}

#[test]
fn session_selectors_search_only_the_selected_codex_thread() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let parent_id = "c0de6101-0000-0000-0000-000000000000";
    let worker_id = "c0de6102-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        parent_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"shared selector marker in the parent"}]}}"#,
        ],
    );
    plant_codex_subagent(
        codex.path(),
        worker_id,
        workdir.path(),
        parent_id,
        &[
            r#"{"timestamp":"2026-08-28T10:02:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"shared selector marker in the worker"}]}}"#,
        ],
    );

    let command = |selector: &str| {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .env("CODEX_SESSION_ID", parent_id)
            .env("CODEX_THREAD_ID", worker_id)
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--codex-dir")
            .arg(codex.path())
            .arg("--session")
            .arg(selector)
            .arg("shared selector marker");
        command
    };

    // `current` searches only the top-level Session.
    command("current")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "shared selector marker in the parent",
        ))
        .stdout(predicates::str::contains("shared selector marker in the worker").not());

    // `current-thread` searches only the calling thread, without
    // --include-subagents.
    command("current-thread")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "shared selector marker in the worker",
        ))
        .stdout(predicates::str::contains("shared selector marker in the parent").not());
}

#[test]
fn session_current_selectors_fail_with_the_resolver_error_when_unavailable() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        r#"{"type":"user","message":{"role":"user","content":"unrelated"}}"#,
    );

    // No Harness identity: `show` and `--session` report the resolver error.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("show")
        .arg("current")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no current Session"));
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("show")
        .arg("current-thread")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no current Session"));
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--session")
        .arg("current-thread")
        .arg("unrelated")
        .assert()
        .failure()
        .stderr(predicates::str::contains("no current Session"));
}

#[test]
fn session_current_selectors_report_ambiguity_like_current() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let claude_id = "11111111-aaaa-bbbb-cccc-ddddeeee0220";
    let codex_id = "c0de6201-0000-0000-0000-000000000000";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        claude_id,
        r#"{"type":"user","message":{"role":"user","content":"claude ambiguity marker"}}"#,
    );
    plant_codex_session(
        codex.path(),
        codex_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex ambiguity marker"}]}}"#,
        ],
    );

    let base = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .env("CLAUDE_CODE_SESSION_ID", claude_id)
            .env("CODEX_SESSION_ID", codex_id)
            .env("CODEX_THREAD_ID", codex_id)
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--codex-dir")
            .arg(codex.path());
        command
    };

    base()
        .arg("show")
        .arg("current-thread")
        .assert()
        .failure()
        .stderr(predicates::str::contains("ambiguous"))
        .stderr(predicates::str::contains("--harness"));
    base()
        .arg("--session")
        .arg("current")
        .arg("ambiguity marker")
        .assert()
        .failure()
        .stderr(predicates::str::contains("ambiguous"))
        .stderr(predicates::str::contains("--harness"));

    // --harness resolves the ambiguity for explicit selectors, as for `current`.
    base()
        .arg("--harness")
        .arg("codex")
        .arg("show")
        .arg("current-thread")
        .assert()
        .success()
        .stdout(predicates::str::contains("codex ambiguity marker"));
}

// --- export: one Session snapshot (issue 19, ADR 0012) ---

#[test]
fn export_markdown_writes_claude_provenance_and_transcript() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let output = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0300";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        &[
            r#"{"type":"ai-title","aiTitle":"Export readable chat"}"#,
            r#"{"type":"user","message":{"role":"user","content":"export marker prompt"},"timestamp":"2026-08-01T10:00:00.000Z","cwd":"E:\\projects\\demo","gitBranch":"main"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"export marker reply"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"error[E0433]: failed to resolve"}]}}"#,
        ]
        .join("\n"),
    );
    let dest = output.path().join("export.md");

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg(&dest)
        .assert()
        .success();

    let text = fs::read_to_string(&dest).unwrap();
    assert!(text.contains("# Export readable chat"), "title: {text}");
    assert!(
        text.contains(session_id),
        "full Session ID, not just the prefix: {text}"
    );
    assert!(text.contains("Harness: claude"), "Harness: {text}");
    assert!(
        text.contains(r"E:\projects\demo"),
        "Project shows real cwd: {text}"
    );
    assert!(
        text.contains("2026-08-01T10:00:00.000Z"),
        "source timestamp: {text}"
    );
    assert!(text.contains("Exported:"), "export timestamp: {text}");
    assert!(
        text.to_lowercase().contains("snapshot"),
        "snapshot status: {text}"
    );
    assert!(text.contains("## Transcript"), "transcript section: {text}");
    assert!(text.contains("export marker prompt"), "prompt: {text}");
    assert!(text.contains("export marker reply"), "reply: {text}");
    assert!(
        text.contains("→ Bash cargo test"),
        "compact tool one-liner: {text}"
    );
    assert!(
        text.contains("✗ Bash FAILED"),
        "failed tool flagged: {text}"
    );
}

#[test]
fn export_markdown_writes_codex_provenance_and_transcript() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let output = tempdir().unwrap();
    let session_id = "c0de7001-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        session_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex export marker prompt"}]}}"#,
            r#"{"timestamp":"2026-08-28T10:02:00.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"codex export marker reply"}]}}"#,
        ],
    );
    plant_codex_title(codex.path(), session_id, "Codex export chat");
    let dest = output.path().join("codex.md");

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg(&dest)
        .assert()
        .success();

    let text = fs::read_to_string(&dest).unwrap();
    assert!(text.contains("# Codex export chat"), "title: {text}");
    assert!(text.contains(session_id), "full Session ID: {text}");
    assert!(text.contains("Harness: codex"), "Harness: {text}");
    assert!(
        text.contains(&workdir.path().to_string_lossy().to_string()),
        "Project: {text}"
    );
    assert!(
        text.contains("2026-08-28T10:02:00.000Z"),
        "source timestamp is the newest record: {text}"
    );
    assert!(text.contains("Exported:"), "export timestamp: {text}");
    assert!(
        text.to_lowercase().contains("snapshot"),
        "snapshot status: {text}"
    );
    assert!(
        text.contains("codex export marker prompt"),
        "prompt: {text}"
    );
    assert!(text.contains("codex export marker reply"), "reply: {text}");
    assert!(text.contains("codex\n"), "codex speaker: {text}");
}

#[test]
fn export_raw_copies_claude_bytes_exactly() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let output = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0310";
    let source = plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        &[
            r#"{"type":"ai-title","aiTitle":"Raw chat"}"#,
            r#"{"type":"user","message":{"role":"user","content":"raw marker"},"timestamp":"2026-08-01T10:00:00.000Z"}"#,
        ]
        .join("\n"),
    );
    let dest = output.path().join("raw.jsonl");

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg(&dest)
        .arg("--format")
        .arg("raw")
        .assert()
        .success();

    assert_eq!(
        fs::read(&dest).unwrap(),
        fs::read(&source).unwrap(),
        "raw Export is byte-identical, no added metadata"
    );
}

#[test]
fn export_raw_copies_codex_bytes_exactly() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let output = tempdir().unwrap();
    let session_id = "c0de7002-0000-0000-0000-000000000000";
    let source = plant_codex_session(
        codex.path(),
        session_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex raw marker"}]}}"#,
        ],
    );
    let dest = output.path().join("codex-raw.jsonl");

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg(&dest)
        .arg("--format")
        .arg("raw")
        .assert()
        .success();

    assert_eq!(
        fs::read(&dest).unwrap(),
        fs::read(&source).unwrap(),
        "raw Codex Export is byte-identical"
    );
}

#[test]
fn export_to_stdout_writes_markdown_without_a_file() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0320";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        &[
            r#"{"type":"ai-title","aiTitle":"Stdout chat"}"#,
            r#"{"type":"user","message":{"role":"user","content":"stdout marker"},"timestamp":"2026-08-01T10:00:00.000Z"}"#,
        ]
        .join("\n"),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg("-")
        .assert()
        .success()
        .stdout(predicates::str::contains("# Stdout chat"))
        .stdout(predicates::str::contains(session_id))
        .stdout(predicates::str::contains("stdout marker"));
}

#[test]
fn export_raw_to_stdout_writes_exact_source_bytes() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0321";
    let source = plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        r#"{"type":"user","message":{"role":"user","content":"raw stdout marker"}}"#,
    );
    let expected = fs::read_to_string(&source).unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg("-")
        .arg("--format")
        .arg("raw")
        .assert()
        .success()
        .stdout(predicates::str::contains("raw stdout marker"))
        .stdout(predicates::str::contains(expected.trim()));
}

#[test]
fn export_markdown_snapshot_ignores_an_incomplete_trailing_record() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let output = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0330";
    let source = plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        r#"{"type":"user","message":{"role":"user","content":"complete marker"},"timestamp":"2026-08-01T10:00:00.000Z"}"#,
    );
    // An active Harness may be mid-write: a truncated final line with no
    // closing braces. Export must finish and render only complete Records.
    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
        writeln!(
            file,
            "\n{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"incomplete"
        )
        .unwrap();
    }
    let dest = output.path().join("snapshot.md");

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg(&dest)
        .assert()
        .success();

    let text = fs::read_to_string(&dest).unwrap();
    assert!(
        text.contains("complete marker"),
        "complete Records are exported: {text}"
    );
    assert!(
        !text.contains("incomplete"),
        "the truncated trailing record is ignored: {text}"
    );
    assert!(
        text.to_lowercase().contains("snapshot"),
        "the document identifies itself as a snapshot: {text}"
    );
}

#[test]
fn export_codex_snapshot_ignores_an_incomplete_trailing_record() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let output = tempdir().unwrap();
    let session_id = "c0de7003-0000-0000-0000-000000000000";
    let source = plant_codex_session(
        codex.path(),
        session_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex complete marker"}]}}"#,
        ],
    );
    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new().append(true).open(&source).unwrap();
        writeln!(
            file,
            "\n{{\"timestamp\":\"2026-08-28T10:02:00.000Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\""
        )
        .unwrap();
    }
    let dest = output.path().join("codex-snapshot.md");

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg(&dest)
        .assert()
        .success();

    let text = fs::read_to_string(&dest).unwrap();
    assert!(
        text.contains("codex complete marker"),
        "complete Records are exported: {text}"
    );
}

#[test]
fn export_refuses_to_overwrite_without_force_and_replaces_with_force() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let output = tempdir().unwrap();
    let session_id = "11111111-aaaa-bbbb-cccc-ddddeeee0340";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        session_id,
        r#"{"type":"user","message":{"role":"user","content":"overwrite marker"},"timestamp":"2026-08-01T10:00:00.000Z"}"#,
    );
    let dest = output.path().join("existing.md");
    fs::write(&dest, "earlier snapshot\n").unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg(&dest)
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"))
        .stderr(predicates::str::contains("--force"));
    assert_eq!(
        fs::read_to_string(&dest).unwrap(),
        "earlier snapshot\n",
        "the existing file is left untouched"
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg(&session_id[..8])
        .arg(&dest)
        .arg("--force")
        .assert()
        .success();
    let replaced = fs::read_to_string(&dest).unwrap();
    assert!(
        replaced.contains("overwrite marker"),
        "explicit --force replaces the file: {replaced}"
    );
}

#[test]
fn export_current_exports_only_the_top_level_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let output = tempdir().unwrap();
    let parent_id = "11111111-aaaa-bbbb-cccc-ddddeeee0350";
    let worker_id = "11111111-aaaa-bbbb-cccc-ddddeeee0351";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        parent_id,
        r#"{"type":"user","message":{"role":"user","content":"parent export marker"}}"#,
    );
    plant_claude_subagent(
        claude.path(),
        workdir.path(),
        parent_id,
        worker_id,
        r#"{"type":"user","message":{"role":"user","content":"worker export marker"}}"#,
    );
    let dest = output.path().join("current.md");

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg("current")
        .arg(&dest)
        .assert()
        .success();

    let text = fs::read_to_string(&dest).unwrap();
    assert!(
        text.contains("parent export marker"),
        "current selects the top-level Session: {text}"
    );
    assert!(
        !text.contains("worker export marker"),
        "it does not bundle the worker family: {text}"
    );
    assert!(text.contains(parent_id), "full top-level ID: {text}");
}

#[test]
fn export_current_thread_exports_the_calling_worker() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let output = tempdir().unwrap();
    let parent_id = "11111111-aaaa-bbbb-cccc-ddddeeee0360";
    let worker_id = "11111111-aaaa-bbbb-cccc-ddddeeee0361";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        parent_id,
        r#"{"type":"user","message":{"role":"user","content":"parent thread marker"}}"#,
    );
    plant_claude_subagent(
        claude.path(),
        workdir.path(),
        parent_id,
        worker_id,
        r#"{"type":"user","message":{"role":"user","content":"worker thread marker"}}"#,
    );
    let dest = output.path().join("thread.md");

    // Explicit current-thread reaches the worker without --include-subagents.
    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg("current-thread")
        .arg(&dest)
        .assert()
        .success();

    let text = fs::read_to_string(&dest).unwrap();
    assert!(
        text.contains("worker thread marker"),
        "current-thread selects the calling thread: {text}"
    );
    assert!(
        !text.contains("parent thread marker"),
        "only the calling thread: {text}"
    );
}

#[test]
fn export_codex_thread_selectors_export_the_intended_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let output = tempdir().unwrap();
    let parent_id = "c0de7101-0000-0000-0000-000000000000";
    let worker_id = "c0de7102-0000-0000-0000-000000000000";
    plant_codex_session(
        codex.path(),
        parent_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex parent export marker"}]}}"#,
        ],
    );
    plant_codex_subagent(
        codex.path(),
        worker_id,
        workdir.path(),
        parent_id,
        &[
            r#"{"timestamp":"2026-08-28T10:02:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex worker export marker"}]}}"#,
        ],
    );

    let current_dest = output.path().join("codex-current.md");
    agsearch_command()
        .current_dir(workdir.path())
        .env("CODEX_SESSION_ID", parent_id)
        .env("CODEX_THREAD_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("export")
        .arg("current")
        .arg(&current_dest)
        .assert()
        .success();
    let current_text = fs::read_to_string(&current_dest).unwrap();
    assert!(
        current_text.contains("codex parent export marker"),
        "current selects the top-level Codex Session: {current_text}"
    );
    assert!(
        !current_text.contains("codex worker export marker"),
        "not the worker family: {current_text}"
    );

    let thread_dest = output.path().join("codex-thread.md");
    agsearch_command()
        .current_dir(workdir.path())
        .env("CODEX_SESSION_ID", parent_id)
        .env("CODEX_THREAD_ID", worker_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("export")
        .arg("current-thread")
        .arg(&thread_dest)
        .assert()
        .success();
    let thread_text = fs::read_to_string(&thread_dest).unwrap();
    assert!(
        thread_text.contains("codex worker export marker"),
        "current-thread selects the worker: {thread_text}"
    );
    assert!(
        !thread_text.contains("codex parent export marker"),
        "only the worker: {thread_text}"
    );
}

#[test]
fn export_reports_unresolved_selectors_and_missing_current_context() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let output = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        r#"{"type":"user","message":{"role":"user","content":"unrelated"}}"#,
    );

    // Unknown prefix.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg("zzz")
        .arg(output.path().join("missing.md"))
        .assert()
        .failure()
        .stderr(predicates::str::contains("no session matches"));

    // No Harness identity for `current`.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("export")
        .arg("current")
        .arg(output.path().join("current.md"))
        .assert()
        .failure()
        .stderr(predicates::str::contains("no current Session"));
}

#[test]
fn export_reports_ambiguity_like_current() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let output = tempdir().unwrap();
    let claude_id = "11111111-aaaa-bbbb-cccc-ddddeeee0370";
    let codex_id = "c0de7201-0000-0000-0000-000000000000";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        claude_id,
        r#"{"type":"user","message":{"role":"user","content":"claude export ambiguity"}}"#,
    );
    plant_codex_session(
        codex.path(),
        codex_id,
        workdir.path(),
        "user",
        &[
            r#"{"timestamp":"2026-08-28T10:01:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex export ambiguity"}]}}"#,
        ],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", claude_id)
        .env("CODEX_SESSION_ID", codex_id)
        .env("CODEX_THREAD_ID", codex_id)
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("export")
        .arg("current")
        .arg(output.path().join("ambiguous.md"))
        .assert()
        .failure()
        .stderr(predicates::str::contains("ambiguous"))
        .stderr(predicates::str::contains("--harness"));
}

// --- --file lists Touches by File Selector (issue 25, Claude Code) ---

/// One assistant `tool_use` line for a Touch-capable tool.
fn touch_use(id: &str, name: &str, input_json: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"{id}","name":"{name}","input":{input_json}}}]}}}}"#
    )
}

#[test]
fn file_lists_touches_grouped_by_session_with_handoff_shape() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        touch_use("t1", "Read", r#"{"file_path":"E:\\p\\x.md"}"#),
        touch_use("t2", "Edit", r#"{"file_path":"docs/x.md"}"#),
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("--file")
        .arg("x.md")
        .assert()
        .success()
        // Same id-and-turn handoff shape as Matches: header + turn brackets.
        .stdout(predicates::str::contains("claude · "))
        .stdout(predicates::str::contains("[1] read Read"))
        .stdout(predicates::str::contains("[2] write Edit"))
        .stdout(predicates::str::contains("E:\\p\\x.md"))
        .stdout(predicates::str::contains("docs/x.md"));
}

#[test]
fn file_suffix_matches_windows_unix_and_relative_paths() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        touch_use("t1", "Read", r#"{"file_path":"E:\\p\\x.md"}"#),
        touch_use("t2", "Read", r#"{"file_path":"/home/u/p/x.md"}"#),
        touch_use("t3", "Read", r#"{"file_path":"docs/x.md"}"#),
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("E:\\p\\x.md"))
        .stdout(predicates::str::contains("/home/u/p/x.md"))
        .stdout(predicates::str::contains("docs/x.md"));
}

#[test]
fn file_multi_segment_selector_does_not_match_a_different_parent() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        touch_use("t1", "Read", r#"{"file_path":"docs/x.md"}"#),
        touch_use("t2", "Read", r#"{"file_path":"other/x.md"}"#),
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("--file")
        .arg("docs/x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("docs/x.md"))
        .stdout(predicates::str::contains("other/x.md").not());
}

#[test]
fn file_matching_is_case_insensitive_and_separator_agnostic() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = touch_use("t1", "Read", r#"{"file_path":"DOCS\\X.MD"}"#);
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    // Lowercase slash selector matches uppercase backslash Touch.
    cmd.arg("--file")
        .arg("docs/x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("DOCS\\X.MD"));
}

#[test]
fn file_absolute_selector_matches_only_that_file() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        touch_use("t1", "Read", r#"{"file_path":"E:\\p\\x.md"}"#),
        touch_use("t2", "Read", r#"{"file_path":"E:\\other\\x.md"}"#),
    ]
    .join("\n");

    // An absolute selector pasted from a tool call selects only that file.
    agsearch_in(workdir.path(), store.path(), &lines)
        .arg("--file")
        .arg("E:\\p\\x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("E:\\p\\x.md"))
        .stdout(predicates::str::contains("E:\\other\\x.md").not());
}

#[test]
fn file_kind_mapping_covers_all_write_tools_and_excludes_non_touch_tools() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        touch_use("r1", "Read", r#"{"file_path":"x.md"}"#),
        touch_use("e1", "Edit", r#"{"file_path":"x.md"}"#),
        touch_use("w1", "Write", r#"{"file_path":"x.md"}"#),
        touch_use("m1", "MultiEdit", r#"{"file_path":"x.md"}"#),
        touch_use("n1", "NotebookEdit", r#"{"notebook_path":"x.md"}"#),
        // Non-Touch tools that must never produce a Touch, even with file-like inputs.
        touch_use("g1", "Glob", r#"{"pattern":"x.md"}"#),
        touch_use("g2", "Grep", r#"{"pattern":"x.md"}"#),
        touch_use("b1", "Bash", r#"{"command":"cat x.md"}"#),
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("-m")
        .arg("0")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("read Read"))
        .stdout(predicates::str::contains("write Edit"))
        .stdout(predicates::str::contains("write Write"))
        .stdout(predicates::str::contains("write MultiEdit"))
        .stdout(predicates::str::contains("write NotebookEdit"))
        .stdout(predicates::str::contains("Glob").not())
        .stdout(predicates::str::contains("Grep").not())
        .stdout(predicates::str::contains("cat x.md").not());
}

#[test]
fn file_failed_touch_is_listed_and_flagged() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        touch_use("t1", "Edit", r#"{"file_path":"x.md"}"#),
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"String to replace not found"}]}}"#.to_string(),
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("write Edit"))
        .stdout(predicates::str::contains("FAILED"));
}

#[test]
fn file_rows_are_capped_by_max_per_session_with_a_show_hint() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        touch_use("t2", "Read", r#"{"file_path":"x.md"}"#),
        touch_use("t3", "Read", r#"{"file_path":"x.md"}"#),
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("-m")
        .arg("1")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("[1] read Read"))
        .stdout(predicates::str::contains("[2]").not())
        .stdout(predicates::str::contains("+2 more"))
        .stdout(predicates::str::contains("agsearch show"));
}

#[test]
fn file_files_flag_prints_only_session_paths() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );

    cmd.arg("-l")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("session.jsonl"))
        .stdout(predicates::str::contains("[1]").not())
        .stdout(predicates::str::contains("read Read").not());
}

#[test]
fn file_groups_by_session_newest_first() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_claude_session(
        store.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        &[
            r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-01-01T10:00:00.000Z"}"#.to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );
    plant_claude_session(
        store.path(),
        workdir.path(),
        "bbbbbbbb-1111-1111-1111-111111111111",
        &[
            r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-06-01T10:00:00.000Z"}"#.to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("aaaaaaaa"))
        .stdout(predicates::str::contains("bbbbbbbb"))
        .stdout(predicates::function::function(|out: &str| {
            out.find("bbbbbbbb").unwrap() < out.find("aaaaaaaa").unwrap()
        }));
}

#[test]
fn file_session_scope_searches_only_the_named_session() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_session(
        store.path(),
        "E--projects-demo",
        "aaaa1111-0000-0000-0000-000000000000",
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );
    plant_session(
        store.path(),
        "E--projects-demo",
        "bbbb2222-0000-0000-0000-000000000000",
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--session")
        .arg("aaaa1111")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("aaaa1111"))
        .stdout(predicates::str::contains("bbbb2222").not());
}

/// Two Sessions with Touches in the current Project, one of which is Current.
/// Short-ids differ so headers are distinguishable.
fn plant_current_and_earlier_touches(
    workdir: &std::path::Path,
    claude: &std::path::Path,
) -> String {
    let current_id = "cccccccc-0000-0000-0000-000000000400";
    plant_claude_session(
        claude,
        workdir,
        current_id,
        &[
            r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-08-02T10:00:00.000Z"}"#.to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );
    plant_claude_session(
        claude,
        workdir,
        "aaaaaaaa-0000-0000-0000-000000000401",
        &[
            r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-08-01T10:00:00.000Z"}"#.to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );
    current_id.to_string()
}

#[test]
fn file_session_current_selects_only_the_current_session() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_touches(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--session")
        .arg("current")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains(&current_id[..8]));
}

#[test]
fn file_excludes_the_current_family_by_default_and_restores_with_include_current() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_touches(workdir.path(), claude.path());

    // Default: only earlier work.
    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains(&current_id[..8]).not());

    // Opt-in: the family returns.
    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--include-current")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains(&current_id[..8]));
}

#[test]
fn file_all_widens_scope_beyond_the_current_project() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_project(
        store.path(),
        "E--projects-other",
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );

    // Default current-project scope sees nothing.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--all")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("E--projects-other"));
}

#[test]
fn file_since_excludes_older_sessions() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let project = store
        .path()
        .join("projects")
        .join(agsearch::encode_project_dir(
            &workdir.path().to_string_lossy(),
        ));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("newish.jsonl"),
        [
            r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-06-01T10:00:00.000Z"}"#,
            &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    )
    .unwrap();
    fs::write(
        project.join("oldie.jsonl"),
        [
            r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-01-01T10:00:00.000Z"}"#,
            &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    )
    .unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--since")
        .arg("2026-05-01")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("newish"))
        .stdout(predicates::str::contains("oldie").not());
}

#[test]
fn file_harness_claude_lists_claude_touches_and_codex_lists_none() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );
    let command = |harness: &str| {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--harness")
            .arg(harness)
            .arg("--file")
            .arg("x.md");
        command
    };

    command("claude")
        .assert()
        .success()
        .stdout(predicates::str::contains("aaaaaaaa"));
    command("codex")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
}

#[test]
fn file_rejects_failed_and_stats_with_a_clear_error() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let _ = agsearch_in(
        workdir.path(),
        store.path(),
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );
    // Build fresh commands: agsearch_in plants once; reuse store for each run.
    let run = |extra: &[&str]| {
        let mut cmd = agsearch_command();
        cmd.current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(store.path())
            .arg("--file")
            .arg("x.md");
        for flag in extra {
            cmd.arg(flag);
        }
        cmd
    };

    run(&["--failed"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
    run(&["--stats"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn file_rejects_a_repeated_flag_with_a_clear_error() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let _ = agsearch_in(
        workdir.path(),
        store.path(),
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .arg("--file")
        .arg("y.md")
        .assert()
        .failure()
        .stderr(predicates::str::contains("--file"));
}

#[test]
fn file_rejects_an_empty_selector_with_a_clear_error() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let _ = agsearch_in(
        workdir.path(),
        store.path(),
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );

    for selector in ["", "   ", "/"] {
        agsearch_command()
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(store.path())
            .arg("--file")
            .arg(selector)
            .assert()
            .failure()
            .stderr(predicates::str::contains("--file"));
    }
}

#[test]
fn file_trailing_separator_is_ignored() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        &touch_use("t1", "Read", r#"{"file_path":"docs/x.md"}"#),
    );

    cmd.arg("--file")
        .arg("x.md/")
        .assert()
        .success()
        .stdout(predicates::str::contains("docs/x.md"));
}

#[test]
fn file_reports_the_standard_empty_message_when_nothing_touches() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let mut cmd = agsearch_in(
        workdir.path(),
        store.path(),
        r#"{"type":"user","message":{"role":"user","content":"no tools here"}}"#,
    );

    cmd.arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
}

#[test]
fn sessions_and_projects_reject_file() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    fs::create_dir_all(store.path().join("projects")).unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("sessions")
        .arg("--file")
        .arg("x.md")
        .assert()
        .failure();
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("projects")
        .arg("--file")
        .arg("x.md")
        .assert()
        .failure();
}

#[test]
fn file_project_flag_targets_a_named_project() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_project(
        store.path(),
        "E--projects-wanted",
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );
    plant_project(
        store.path(),
        "E--projects-other",
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--project")
        .arg("wanted")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("E--projects-wanted"))
        .stdout(predicates::str::contains("E--projects-other").not());
}

// --- --written narrows --file to write Touches (issue 26) ---

#[test]
fn written_drops_read_rows_but_keeps_write_tools() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        touch_use("r1", "Read", r#"{"file_path":"x.md"}"#),
        touch_use("e1", "Edit", r#"{"file_path":"x.md"}"#),
        touch_use("w1", "Write", r#"{"file_path":"x.md"}"#),
        touch_use("m1", "MultiEdit", r#"{"file_path":"x.md"}"#),
        touch_use("n1", "NotebookEdit", r#"{"notebook_path":"x.md"}"#),
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("-m")
        .arg("0")
        .arg("--file")
        .arg("x.md")
        .arg("--written")
        .assert()
        .success()
        .stdout(predicates::str::contains("read Read").not())
        .stdout(predicates::str::contains("write Edit"))
        .stdout(predicates::str::contains("write Write"))
        .stdout(predicates::str::contains("write MultiEdit"))
        .stdout(predicates::str::contains("write NotebookEdit"));
}

#[test]
fn written_drops_sessions_left_with_no_rows() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_claude_session(
        store.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );
    plant_claude_session(
        store.path(),
        workdir.path(),
        "bbbbbbbb-1111-1111-1111-111111111111",
        &touch_use("t1", "Edit", r#"{"file_path":"x.md"}"#),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .arg("--written")
        .assert()
        .success()
        .stdout(predicates::str::contains("bbbbbbbb"))
        .stdout(predicates::str::contains("aaaaaaaa").not());
}

#[test]
fn written_without_file_errors_clearly() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let _ = agsearch_in(
        workdir.path(),
        store.path(),
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--written")
        .assert()
        .failure()
        .stderr(predicates::str::contains("--written"));
}

#[test]
fn written_composes_with_files_flag_and_max_per_session() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // One read-only session (drops out under --written) plus one session
    // with a read and two writes (read drops, writes cap at 1).
    plant_claude_session(
        store.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );
    plant_claude_session(
        store.path(),
        workdir.path(),
        "bbbbbbbb-1111-1111-1111-111111111111",
        &[
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
            touch_use("t2", "Edit", r#"{"file_path":"x.md"}"#),
            touch_use("t3", "Write", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );

    // -l lists only the session left with writes (session file paths only,
    // so assert on paths present/absent, not turn brackets).
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("-l")
        .arg("--file")
        .arg("x.md")
        .arg("--written")
        .assert()
        .success()
        .stdout(predicates::str::contains("bbbbbbbb"))
        .stdout(predicates::str::contains("aaaaaaaa").not());

    // -m caps the surviving write rows with the standard hint.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("-m")
        .arg("1")
        .arg("--file")
        .arg("x.md")
        .arg("--written")
        .assert()
        .success()
        .stdout(predicates::str::contains("read Read").not())
        .stdout(predicates::str::contains("+1 more"))
        .stdout(predicates::str::contains("agsearch show"));
}

#[test]
fn written_composes_with_session_current() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = plant_current_and_earlier_touches(workdir.path(), claude.path());

    // The current session's only Touch is a read, so --written finds nothing
    // there while --file alone lists it.
    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("--session")
        .arg("current")
        .arg("--file")
        .arg("x.md")
        .arg("--written")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
}

#[test]
fn written_keeps_writes_in_session_current() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let current_id = "cccccccc-0000-0000-0000-000000000400";
    plant_claude_session(
        claude.path(),
        workdir.path(),
        current_id,
        &[
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
            touch_use("t2", "Edit", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );

    agsearch_as_current(workdir.path(), claude.path(), current_id)
        .arg("--session")
        .arg("current")
        .arg("--file")
        .arg("x.md")
        .arg("--written")
        .assert()
        .success()
        .stdout(predicates::str::contains("write Edit"))
        .stdout(predicates::str::contains("read Read").not());
}

// --- --file combined with a Query restricts Matches to touching Sessions (issue 27) ---

#[test]
fn file_query_returns_matches_only_from_touching_sessions() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_claude_session(
        store.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        &[
            r#"{"type":"user","message":{"role":"user","content":"tokio in touching session"}}"#
                .to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );
    plant_claude_session(
        store.path(),
        workdir.path(),
        "bbbbbbbb-1111-1111-1111-111111111111",
        &[
            r#"{"type":"user","message":{"role":"user","content":"tokio in other session"}}"#
                .to_string(),
            touch_use("t1", "Read", r#"{"file_path":"y.md"}"#),
        ]
        .join("\n"),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("tokio in touching session"))
        .stdout(predicates::str::contains("tokio in other session").not());
}

#[test]
fn file_query_omits_touching_sessions_with_no_match() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_claude_session(
        store.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        &[
            r#"{"type":"user","message":{"role":"user","content":"tokio here"}}"#.to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );
    plant_claude_session(
        store.path(),
        workdir.path(),
        "bbbbbbbb-1111-1111-1111-111111111111",
        &[
            r#"{"type":"user","message":{"role":"user","content":"unrelated content"}}"#
                .to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("aaaaaaaa"))
        .stdout(predicates::str::contains("bbbbbbbb").not())
        .stdout(predicates::str::contains("tokio here"));
}

#[test]
fn file_query_output_uses_the_standard_match_shape() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"tokio needle here"}}"#.to_string(),
        touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    ]
    .join("\n");
    let mut cmd = agsearch_in(workdir.path(), store.path(), &lines);

    cmd.arg("--file")
        .arg("x.md")
        .arg("tokio")
        .assert()
        .success()
        // Same header + turn + role + snippet shape as a plain search, not the
        // Touch row shape (`read Read <path>` never appears).
        .stdout(predicates::str::contains("claude · "))
        .stdout(predicates::str::contains("[1] user:"))
        .stdout(predicates::str::contains("tokio needle here"))
        .stdout(predicates::str::contains("read Read").not());
}

#[test]
fn file_query_written_restricts_to_write_touches() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_claude_session(
        store.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        &[
            r#"{"type":"user","message":{"role":"user","content":"tokio read touch"}}"#.to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );
    plant_claude_session(
        store.path(),
        workdir.path(),
        "bbbbbbbb-1111-1111-1111-111111111111",
        &[
            r#"{"type":"user","message":{"role":"user","content":"tokio write touch"}}"#
                .to_string(),
            touch_use("t1", "Edit", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );

    // Without --written both touching Sessions match.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("aaaaaaaa"))
        .stdout(predicates::str::contains("bbbbbbbb"));

    // With --written only the Session with a write Touch remains.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .arg("--written")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("bbbbbbbb"))
        .stdout(predicates::str::contains("aaaaaaaa").not())
        .stdout(predicates::str::contains("tokio write touch"));
}

#[test]
fn file_query_keeps_thinking_and_tools_meaning() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // One Session whose only Match is in thinking, one whose only Match is in
    // tool content; both touch the selected file.
    plant_claude_session(
        store.path(),
        workdir.path(),
        "aaaaaaaa-0000-0000-0000-000000000000",
        &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"pondering the zylophone problem"}]}}"#.to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );
    plant_claude_session(
        store.path(),
        workdir.path(),
        "bbbbbbbb-1111-1111-1111-111111111111",
        &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"zylophone --tune"}}]}}"#.to_string(),
            touch_use("t2", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );

    // By default neither thinking nor tool content matches.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--file")
        .arg("x.md")
        .arg("zylophone")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));

    // --thinking finds only the thinking Session.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--thinking")
        .arg("--file")
        .arg("x.md")
        .arg("zylophone")
        .assert()
        .success()
        .stdout(predicates::str::contains("aaaaaaaa"))
        .stdout(predicates::str::contains("bbbbbbbb").not())
        .stdout(predicates::str::contains("thinking"));

    // --tools finds only the tool Session; --all-content finds both.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--tools")
        .arg("--file")
        .arg("x.md")
        .arg("zylophone")
        .assert()
        .success()
        .stdout(predicates::str::contains("bbbbbbbb"))
        .stdout(predicates::str::contains("aaaaaaaa").not());
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--all-content")
        .arg("--file")
        .arg("x.md")
        .arg("zylophone")
        .assert()
        .success()
        .stdout(predicates::str::contains("aaaaaaaa"))
        .stdout(predicates::str::contains("bbbbbbbb"));
}

#[test]
fn file_query_keeps_regex_and_case_meaning() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"reading the borrow checker docs"}}"#
            .to_string(),
        touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    ]
    .join("\n");
    let _ = agsearch_in(workdir.path(), store.path(), &lines);

    let run = |extra: &[&str], query: &str| {
        let mut cmd = agsearch_command();
        cmd.current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(store.path());
        for flag in extra {
            cmd.arg(flag);
        }
        cmd.arg("--file").arg("x.md").arg(query);
        cmd
    };

    // Literal mode treats the pattern as plain text.
    run(&[], "bo+rrow")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
    // Regex mode matches the pattern, still restricted to touching Sessions.
    run(&["--regex"], "bo+rrow")
        .assert()
        .success()
        .stdout(predicates::str::contains("borrow checker"));
    // Case-insensitive by default, sensitive with -s.
    run(&[], "BORROW")
        .assert()
        .success()
        .stdout(predicates::str::contains("borrow checker"));
    run(&["--case-sensitive"], "BORROW")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
}

#[test]
fn file_query_composes_with_session_limit_and_files_flag() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    plant_session(
        store.path(),
        "E--projects-demo",
        "aaaa1111-0000-0000-0000-000000000000",
        &[
            r#"{"type":"user","message":{"role":"user","content":"tokio in session A"}}"#
                .to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );
    plant_session(
        store.path(),
        "E--projects-demo",
        "bbbb2222-0000-0000-0000-000000000000",
        &[
            r#"{"type":"user","message":{"role":"user","content":"tokio in session B"}}"#
                .to_string(),
            touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
        ]
        .join("\n"),
    );

    // --session narrows the combined search to one Session.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--session")
        .arg("aaaa1111")
        .arg("--file")
        .arg("x.md")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("tokio in session A"))
        .stdout(predicates::str::contains("tokio in session B").not());

    // -l prints only the touching Session paths with Matches.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--all")
        .arg("-l")
        .arg("--file")
        .arg("x.md")
        .arg("tokio")
        .assert()
        .success()
        .stdout(predicates::str::contains("aaaa1111"))
        .stdout(predicates::str::contains("bbbb2222"))
        .stdout(predicates::str::contains("tokio in session").not());

    // -m caps the Matches per Session with the standard hint.
    let capped_dir = tempdir().unwrap();
    let capped_store = tempdir().unwrap();
    let capped_lines = [
        r#"{"type":"user","message":{"role":"user","content":"zebra one"}}"#.to_string(),
        r#"{"type":"user","message":{"role":"user","content":"zebra two"}}"#.to_string(),
        touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    ]
    .join("\n");
    let _ = agsearch_in(capped_dir.path(), capped_store.path(), &capped_lines);
    let mut capped = agsearch_command();
    capped
        .current_dir(capped_dir.path())
        .arg("--claude-dir")
        .arg(capped_store.path())
        .arg("-m")
        .arg("1")
        .arg("--file")
        .arg("x.md")
        .arg("zebra");
    capped
        .assert()
        .success()
        .stdout(predicates::str::contains("zebra one"))
        .stdout(predicates::str::contains("+1 more"))
        .stdout(predicates::str::contains("agsearch show"));
}

// --- Codex apply_patch produces write Touches (issue 28) ---

/// One Codex `apply_patch` tool call carrying the raw patch string in its
/// input, mirroring real rollouts (`payload.input: "*** Begin Patch\n..."`).
fn codex_apply_patch(call_id: &str, patch: &str) -> String {
    serde_json::json!({
        "timestamp": "2026-08-28T10:03:00.000Z",
        "type": "response_item",
        "payload": {
            "type": "custom_tool_call",
            "call_id": call_id,
            "name": "apply_patch",
            "input": patch,
        }
    })
    .to_string()
}

/// Same patch via the `function_call.arguments` shape the harness also reads.
fn codex_apply_patch_via_arguments(call_id: &str, patch: &str) -> String {
    serde_json::json!({
        "timestamp": "2026-08-28T10:03:00.000Z",
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "call_id": call_id,
            "name": "apply_patch",
            "arguments": patch,
        }
    })
    .to_string()
}

/// One Codex shell tool call (never a Touch, even with a file-like command).
fn codex_shell(call_id: &str, cmd: &str) -> String {
    let input = serde_json::json!({"cmd": cmd}).to_string();
    serde_json::json!({
        "timestamp": "2026-08-28T10:03:00.000Z",
        "type": "response_item",
        "payload": {
            "type": "custom_tool_call",
            "call_id": call_id,
            "name": "shell",
            "input": input,
        }
    })
    .to_string()
}

fn codex_output(call_id: &str, output: serde_json::Value) -> String {
    serde_json::json!({
        "timestamp": "2026-08-28T10:04:00.000Z",
        "type": "response_item",
        "payload": {
            "type": "custom_tool_call_output",
            "call_id": call_id,
            "output": output,
        }
    })
    .to_string()
}

fn codex_message(text: &str) -> String {
    serde_json::json!({
        "timestamp": "2026-08-28T10:05:00.000Z",
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text}],
        }
    })
    .to_string()
}

fn codex_file_command(workdir: &std::path::Path, codex_dir: &std::path::Path) -> Command {
    let mut cmd = agsearch_command();
    cmd.current_dir(workdir)
        .arg("--claude-dir")
        .arg(workdir.join("missing-claude"))
        .arg("--codex-dir")
        .arg(codex_dir)
        .arg("--harness")
        .arg("codex");
    cmd
}

#[test]
fn codex_apply_patch_touching_two_files_yields_two_write_touches() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let patch = "*** Begin Patch\n*** Add File: a/x.md\n+hello\n*** Update File: b/x.md\n@@\n-old\n+new\n*** End Patch\n";
    let call = codex_apply_patch("patch-1", patch);
    let done = codex_message("done");
    plant_codex_session(
        codex.path(),
        "c0de0028-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[call.as_str(), done.as_str()],
    );

    codex_file_command(workdir.path(), codex.path())
        .arg("-m")
        .arg("0")
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("codex · c0de0028"))
        .stdout(predicates::str::contains("write apply_patch"))
        .stdout(predicates::str::contains("a/x.md"))
        .stdout(predicates::str::contains("b/x.md"));

    // Write Touches survive --written.
    codex_file_command(workdir.path(), codex.path())
        .arg("--file")
        .arg("x.md")
        .arg("--written")
        .assert()
        .success()
        .stdout(predicates::str::contains("a/x.md"))
        .stdout(predicates::str::contains("b/x.md"));
}

#[test]
fn codex_apply_patch_add_update_and_delete_hunks_all_count() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let patch = "*** Begin Patch\n*** Add File: docs/added.md\n+new\n*** Update File: docs/changed.md\n@@\n-old\n+new\n*** Delete File: docs/removed.md\n*** End Patch\n";
    // Exercise the `function_call.arguments` input shape here (the other
    // Codex tests use `custom_tool_call.input`).
    let call = codex_apply_patch_via_arguments("patch-1", patch);
    let done = codex_message("done");
    plant_codex_session(
        codex.path(),
        "c0de0029-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[call.as_str(), done.as_str()],
    );

    for selector in ["added.md", "changed.md", "removed.md"] {
        codex_file_command(workdir.path(), codex.path())
            .arg("--file")
            .arg(selector)
            .assert()
            .success()
            .stdout(predicates::str::contains("codex · c0de0029"))
            .stdout(predicates::str::contains("write apply_patch"));
    }
}

#[test]
fn codex_shell_calls_produce_no_touch() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let shell = codex_shell("shell-1", "cat x.md");
    let done = codex_message("done");
    plant_codex_session(
        codex.path(),
        "c0de0030-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[shell.as_str(), done.as_str()],
    );

    codex_file_command(workdir.path(), codex.path())
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("No matches."));
}

#[test]
fn codex_failed_apply_patch_is_listed_and_flagged() {
    let workdir = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let patch = "*** Begin Patch\n*** Update File: docs/x.md\n@@\n-old\n+new\n*** End Patch\n";
    let call = codex_apply_patch("patch-1", patch);
    let output = codex_output(
        "patch-1",
        serde_json::json!({"exit_code": 1, "output": "patch failed"}),
    );
    let done = codex_message("done");
    plant_codex_session(
        codex.path(),
        "c0de0031-0000-0000-0000-000000000000",
        workdir.path(),
        "user",
        &[call.as_str(), output.as_str(), done.as_str()],
    );

    codex_file_command(workdir.path(), codex.path())
        .arg("--file")
        .arg("x.md")
        .assert()
        .success()
        .stdout(predicates::str::contains("write apply_patch"))
        .stdout(predicates::str::contains("FAILED"));
}

// --- usage: per-call breakdown for one Claude Code Session (issue 30) ---

/// Build one Claude `assistant` line carrying Usage, for usage-breakdown fixtures.
fn claude_usage_assistant(
    message_id: &str,
    timestamp: &str,
    text: &str,
    input: u64,
    cache_create: u64,
    cache_read: u64,
    output: u64,
) -> String {
    serde_json::json!({
        "type": "assistant",
        "timestamp": timestamp,
        "message": {
            "role": "assistant",
            "model": "claude-fable-5",
            "id": message_id,
            "content": [{"type": "text", "text": text}],
            "usage": {
                "input_tokens": input,
                "cache_creation_input_tokens": cache_create,
                "cache_read_input_tokens": cache_read,
                "output_tokens": output
            }
        }
    })
    .to_string()
}

fn claude_user_prompt(text: &str, timestamp: &str) -> String {
    serde_json::json!({
        "type": "user",
        "timestamp": timestamp,
        "message": {"role": "user", "content": text}
    })
    .to_string()
}

#[test]
fn usage_breakdown_lists_calls_in_turn_order_with_total() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let session_id = "aaaaaaaa-1111-2222-3333-444444444444";
    plant_claude_session(
        store.path(),
        workdir.path(),
        session_id,
        &[
            claude_user_prompt("first prompt", "2026-08-20T04:00:00.000Z"),
            claude_usage_assistant(
                "msg_0001",
                "2026-08-20T04:00:10.000Z",
                "first call preview alpha",
                10,
                2000,
                30000,
                50,
            ),
            claude_user_prompt("second prompt", "2026-08-20T04:00:15.000Z"),
            claude_usage_assistant(
                "msg_0002",
                "2026-08-20T04:00:20.000Z",
                "second call preview beta",
                5,
                500,
                40000,
                12000,
            ),
        ]
        .join("\n"),
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg(&session_id[..8])
        .assert()
        .success()
        // Header plus both previews and the total line.
        .stdout(predicates::str::contains("turn"))
        .stdout(predicates::str::contains("first call preview alpha"))
        .stdout(predicates::str::contains("second call preview beta"))
        .stdout(predicates::str::contains("total"))
        .stdout(predicates::str::contains("2 calls"))
        // Thousands separators on the large counts.
        .stdout(predicates::str::contains("30,000"))
        .stdout(predicates::str::contains("40,000"))
        .stdout(predicates::str::contains("12,000"))
        // Totals: input 15, cache-write 2,500, cache-read 70,000,
        // output 12,050, total 84,565.
        .stdout(predicates::str::contains("2,500"))
        .stdout(predicates::str::contains("70,000"))
        .stdout(predicates::str::contains("84,565"))
        // Default is turn order: the first call precedes the second.
        .stdout(predicates::function::function(|out: &str| {
            out.find("first call preview alpha").unwrap()
                < out.find("second call preview beta").unwrap()
        }));
}

#[test]
fn usage_breakdown_counts_shared_message_id_once() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let session_id = "bbbbbbbb-1111-2222-3333-444444444444";
    // One API call written as three Records sharing one message.id (thinking,
    // text, tool_use) with identical Usage, plus a second distinct call.
    let shared_id = "msg_shared_0001";
    let thinking = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-08-20T04:00:10.000Z",
        "message": {
            "role": "assistant",
            "model": "claude-fable-5",
            "id": shared_id,
            "content": [{"type": "thinking", "thinking": "planning the fix"}],
            "usage": {"input_tokens": 5, "cache_creation_input_tokens": 500, "cache_read_input_tokens": 40000, "output_tokens": 12000}
        }
    })
    .to_string();
    let text = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-08-20T04:00:11.000Z",
        "message": {
            "role": "assistant",
            "model": "claude-fable-5",
            "id": shared_id,
            "content": [{"type": "text", "text": "shared call preview gamma"}],
            "usage": {"input_tokens": 5, "cache_creation_input_tokens": 500, "cache_read_input_tokens": 40000, "output_tokens": 12000}
        }
    })
    .to_string();
    let tool = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-08-20T04:00:12.000Z",
        "message": {
            "role": "assistant",
            "model": "claude-fable-5",
            "id": shared_id,
            "content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "cargo test"}}],
            "usage": {"input_tokens": 5, "cache_creation_input_tokens": 500, "cache_read_input_tokens": 40000, "output_tokens": 12000}
        }
    })
    .to_string();
    plant_claude_session(
        store.path(),
        workdir.path(),
        session_id,
        &[
            claude_user_prompt("prompt", "2026-08-20T04:00:00.000Z"),
            thinking,
            text,
            tool,
            claude_usage_assistant(
                "msg_0002",
                "2026-08-20T04:00:20.000Z",
                "other call preview delta",
                10,
                2000,
                30000,
                50,
            ),
        ]
        .join("\n"),
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg(&session_id[..8])
        .assert()
        .success()
        // The shared call appears once (its first text block wins the preview).
        .stdout(predicates::str::contains("shared call preview gamma"))
        .stdout(predicates::str::contains("other call preview delta"))
        .stdout(predicates::str::contains("2 calls"))
        // Counted once: input 5+10=15, cache-write 500+2000=2,500,
        // cache-read 40000+30000=70,000, output 12000+50=12,050.
        // Triple-counting would give 25 / 3,500 / 150,000 / 36,050.
        .stdout(predicates::str::contains("2,500"))
        .stdout(predicates::str::contains("70,000"))
        .stdout(predicates::str::contains("84,565"))
        .stdout(predicates::function::function(|out: &str| {
            out.matches("shared call preview gamma").count() == 1
        }));
}

#[test]
fn usage_breakdown_shows_blank_cells_for_calls_without_usage() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let session_id = "cccccccc-1111-2222-3333-444444444444";
    let no_usage = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-08-20T04:00:30.000Z",
        "message": {
            "role": "assistant",
            "model": "claude-fable-5",
            "id": "msg_no_usage",
            "content": [{"type": "text", "text": "no usage marker epsilon"}]
        }
    })
    .to_string();
    plant_claude_session(
        store.path(),
        workdir.path(),
        session_id,
        &[
            claude_user_prompt("prompt", "2026-08-20T04:00:00.000Z"),
            claude_usage_assistant(
                "msg_0001",
                "2026-08-20T04:00:10.000Z",
                "usage marker zeta",
                12345,
                2000,
                30000,
                50,
            ),
            no_usage,
        ]
        .join("\n"),
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg(&session_id[..8])
        .assert()
        .success()
        .stdout(predicates::str::contains("usage marker zeta"))
        .stdout(predicates::str::contains("no usage marker epsilon"))
        // The usage-bearing call uses thousands separators.
        .stdout(predicates::str::contains("12,345"))
        // Totals sum only the usage-bearing call.
        .stdout(predicates::str::contains("2 calls"))
        // The no-usage row shows its model and preview but no comma-formatted
        // numbers: its line carries no ',' (timestamps, turns and model names
        // never contain commas; only thousands separators do).
        .stdout(predicates::function::function(|out: &str| {
            out.lines()
                .find(|line| line.contains("no usage marker epsilon"))
                .is_some_and(|line| !line.contains(','))
        }));
}

#[test]
fn usage_breakdown_sort_total_orders_biggest_first() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let session_id = "dddddddd-1111-2222-3333-444444444444";
    plant_claude_session(
        store.path(),
        workdir.path(),
        session_id,
        &[
            claude_user_prompt("prompt", "2026-08-20T04:00:00.000Z"),
            claude_usage_assistant(
                "msg_small",
                "2026-08-20T04:00:10.000Z",
                "small call preview",
                10,
                2000,
                30000,
                50,
            ),
            claude_usage_assistant(
                "msg_big",
                "2026-08-20T04:00:20.000Z",
                "big call preview",
                5,
                500,
                40000,
                12000,
            ),
        ]
        .join("\n"),
    );

    // Default: turn order (small before big).
    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg(&session_id[..8])
        .assert()
        .success()
        .stdout(predicates::function::function(|out: &str| {
            out.find("small call preview").unwrap() < out.find("big call preview").unwrap()
        }));

    // --sort total: biggest first (big before small).
    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg(&session_id[..8])
        .arg("--sort")
        .arg("total")
        .assert()
        .success()
        .stdout(predicates::function::function(|out: &str| {
            out.find("big call preview").unwrap() < out.find("small call preview").unwrap()
        }));
}

#[test]
fn usage_current_and_current_thread_resolve_through_the_central_resolver() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let parent_id = "eeeeeeee-aaaa-bbbb-cccc-ddddeeee0200";
    let worker_id = "eeeeeeee-aaaa-bbbb-cccc-ddddeeee0201";
    plant_claude_session(
        store.path(),
        workdir.path(),
        parent_id,
        &[
            claude_user_prompt("parent prompt", "2026-08-20T04:00:00.000Z"),
            claude_usage_assistant(
                "msg_parent",
                "2026-08-20T04:00:10.000Z",
                "parent usage marker",
                10,
                2000,
                30000,
                50,
            ),
        ]
        .join("\n"),
    );
    plant_claude_subagent(
        store.path(),
        workdir.path(),
        parent_id,
        worker_id,
        &[
            claude_user_prompt("worker prompt", "2026-08-20T04:00:00.000Z"),
            claude_usage_assistant(
                "msg_worker",
                "2026-08-20T04:00:10.000Z",
                "worker usage marker",
                7,
                1000,
                20000,
                30,
            ),
        ]
        .join("\n"),
    );

    // `current` selects the top-level Session even when invoked from a worker.
    // Since issue 33 the parent breakdown folds its workers: both markers
    // appear, the worker row is marked in the `subagent` column, and the total
    // sums the family (32,060 + 21,037 = 53,097, 2 calls).
    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", worker_id)
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg("current")
        .assert()
        .success()
        .stdout(predicates::str::contains("parent usage marker"))
        .stdout(predicates::str::contains("worker usage marker"))
        .stdout(predicates::str::contains("subagent"))
        .stdout(predicates::str::contains("53,097"))
        .stdout(predicates::str::contains("2 calls"));

    // `current-thread` selects the calling thread without --include-subagents.
    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", worker_id)
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg("current-thread")
        .assert()
        .success()
        .stdout(predicates::str::contains("worker usage marker"))
        .stdout(predicates::str::contains("parent usage marker").not());
}

#[test]
fn usage_without_a_selector_ranks_instead_of_erroring() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains("No sessions with usage."));
}

// --- usage: Codex token_count Records feed the breakdown (issue 31) ---

/// Build one Codex `event_msg` `token_count` line. `last_*` is the per-call
/// usage; `total_*` is the cumulative running total carried by that Record
/// (for non-final Records, pass the same as `last_*`; for the final Record,
/// pass the sums to avoid a mismatch warning, or differing values to trigger
/// one). `total_tokens` is derived as `input + output` (cached and
/// cache-write are subsets of input, matching real Codex files), so callers
/// need not compute it.
#[allow(clippy::too_many_arguments)]
fn codex_token_count(
    timestamp: &str,
    last_input: u64,
    last_cache_write: u64,
    last_cached: u64,
    last_output: u64,
    total_input: u64,
    total_cache_write: u64,
    total_cached: u64,
    total_output: u64,
) -> String {
    let last_total = last_input.saturating_add(last_output);
    let total_total = total_input.saturating_add(total_output);
    serde_json::json!({
        "timestamp": timestamp,
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "total_token_usage": {
                    "input_tokens": total_input,
                    "cache_write_input_tokens": total_cache_write,
                    "cached_input_tokens": total_cached,
                    "output_tokens": total_output,
                    "reasoning_output_tokens": 0,
                    "total_tokens": total_total
                },
                "last_token_usage": {
                    "input_tokens": last_input,
                    "cache_write_input_tokens": last_cache_write,
                    "cached_input_tokens": last_cached,
                    "output_tokens": last_output,
                    "reasoning_output_tokens": 0,
                    "total_tokens": last_total
                },
                "model_context_window": null
            },
            "rate_limits": null
        }
    })
    .to_string()
}

/// Plant a Codex Session whose `session_meta` carries `model` when `Some`,
/// for usage-breakdown fixtures. Uses the same rollout layout as
/// [`plant_codex_session`] but includes the model name the breakdown shows.
fn plant_codex_usage_session(
    codex_dir: &std::path::Path,
    id: &str,
    cwd: &std::path::Path,
    model: Option<&str>,
    records: &[&str],
) -> std::path::PathBuf {
    let mut payload = serde_json::json!({
        "id": id,
        "session_id": id,
        "cwd": cwd.to_string_lossy(),
        "thread_source": "user",
        "source": "cli"
    });
    if let Some(model) = model {
        payload["model"] = serde_json::Value::String(model.to_string());
    }
    let meta = serde_json::json!({
        "timestamp": "2026-08-28T10:00:00.000Z",
        "type": "session_meta",
        "payload": payload
    });
    write_codex_rollout(codex_dir, id, meta, records)
}

fn codex_user_message(text: &str, timestamp: &str) -> String {
    serde_json::json!({
        "timestamp": timestamp,
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}]
        }
    })
    .to_string()
}

fn codex_assistant_message(text: &str, timestamp: &str) -> String {
    serde_json::json!({
        "timestamp": timestamp,
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text}]
        }
    })
    .to_string()
}

#[test]
fn usage_breakdown_codex_sums_token_counts_with_same_columns() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let session_id = "c0de0001-1111-2222-3333-444444444444";
    // Two calls. Input column shows non-cached input for cross-Harness
    // comparability (raw input minus cached minus cache-write), so:
    // call 1: 12345-2000-500=9845, total 9845+500+2000+50=12395;
    // call 2: 40000-30000-2000=8000, total 8000+2000+30000+12000=52000.
    // Sums: input 17845, cache-write 2500, cache-read 32000, output 12050,
    // total 64395. Final total_token_usage carries those sums (no warning).
    // Messages are planted to prove they are ignored when token_counts exist.
    let tc1 = codex_token_count(
        "2026-08-28T10:01:00.000Z",
        12345,
        500,
        2000,
        50,
        12345,
        500,
        2000,
        50,
    );
    let tc2 = codex_token_count(
        "2026-08-28T10:02:00.000Z",
        40000,
        2000,
        30000,
        12000,
        52345,
        2500,
        32000,
        12050,
    );
    plant_codex_usage_session(
        codex.path(),
        session_id,
        workdir.path(),
        Some("codex-fable-7"),
        &[
            &codex_user_message("codex prompt", "2026-08-28T10:00:10.000Z"),
            &codex_assistant_message("codex reply", "2026-08-28T10:00:20.000Z"),
            &tc1,
            &tc2,
        ],
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("usage")
        .arg(&session_id[..8])
        .assert()
        .success()
        // Same columns as the Claude Code breakdown.
        .stdout(predicates::str::contains("turn"))
        .stdout(predicates::str::contains("timestamp"))
        .stdout(predicates::str::contains("model"))
        .stdout(predicates::str::contains("input"))
        .stdout(predicates::str::contains("cache-write"))
        .stdout(predicates::str::contains("cache-read"))
        .stdout(predicates::str::contains("output"))
        .stdout(predicates::str::contains("total"))
        .stdout(predicates::str::contains("preview"))
        // Model from the session meta, timestamps from the token_counts.
        .stdout(predicates::str::contains("codex-fable-7"))
        .stdout(predicates::str::contains("2026-08-28T10:01:00"))
        .stdout(predicates::str::contains("2026-08-28T10:02:00"))
        // Only token_counts become calls (Messages ignored): 2 calls, not 3+.
        .stdout(predicates::str::contains("2 calls"))
        // Thousands separators on the summed totals.
        .stdout(predicates::str::contains("17,845"))
        .stdout(predicates::str::contains("2,500"))
        .stdout(predicates::str::contains("32,000"))
        .stdout(predicates::str::contains("12,050"))
        .stdout(predicates::str::contains("64,395"))
        // Default is turn order (file order): first token_count first.
        .stdout(predicates::function::function(|out: &str| {
            out.find("2026-08-28T10:01:00").unwrap() < out.find("2026-08-28T10:02:00").unwrap()
        }));
}

#[test]
fn usage_breakdown_codex_mismatch_warns_and_keeps_sum() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let session_id = "c0de0002-1111-2222-3333-444444444444";
    let tc1 = codex_token_count(
        "2026-08-28T10:01:00.000Z",
        12345,
        500,
        2000,
        50,
        12345,
        500,
        2000,
        50,
    );
    // Final total disagrees with the sums (input 99999 vs 52345): warn and
    // keep the sums.
    let tc2 = codex_token_count(
        "2026-08-28T10:02:00.000Z",
        40000,
        2000,
        30000,
        12000,
        99999,
        2500,
        32000,
        12050,
    );
    plant_codex_usage_session(
        codex.path(),
        session_id,
        workdir.path(),
        Some("codex-fable-7"),
        &[&tc1, &tc2],
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("usage")
        .arg(&session_id[..8])
        .assert()
        .success()
        .stderr(predicates::str::contains("warning"))
        .stderr(predicates::str::contains("total_token_usage"))
        // Sums kept, not the mismatched final (52345 -> 17845 displayed).
        .stdout(predicates::str::contains("64,395"))
        .stdout(predicates::str::contains("17,845"))
        .stdout(predicates::str::contains("2 calls"));
}

#[test]
fn usage_breakdown_codex_empty_with_no_token_counts() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let session_id = "c0de0003-1111-2222-3333-444444444444";
    // Messages only: no token_count Records -> empty table, zero total (unlike
    // Claude, whose Messages become blank-usage rows).
    plant_codex_usage_session(
        codex.path(),
        session_id,
        workdir.path(),
        Some("codex-fable-7"),
        &[
            &codex_user_message("prompt without usage", "2026-08-28T10:00:10.000Z"),
            &codex_assistant_message("reply without usage", "2026-08-28T10:00:20.000Z"),
        ],
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("usage")
        .arg(&session_id[..8])
        .assert()
        .success()
        .stdout(predicates::str::contains("turn"))
        .stdout(predicates::str::contains("total"))
        .stdout(predicates::str::contains("0 calls"));
}

// --- usage: rank Sessions by Usage with scope, sort and limit (issue 32) ---

/// Plant a Claude Session with one usage-bearing call and an ai-title, for
/// ranking fixtures. Returns the session id prefix assertion helper data via
/// the planted file; the caller keeps the id.
#[allow(clippy::too_many_arguments)]
fn plant_ranking_claude(
    store: &std::path::Path,
    cwd: &std::path::Path,
    session_id: &str,
    title: &str,
    message_id: &str,
    timestamp: &str,
    input: u64,
    cache_create: u64,
    cache_read: u64,
    output: u64,
) {
    let title_line = serde_json::json!({"type":"ai-title","aiTitle":title}).to_string();
    plant_claude_session(
        store,
        cwd,
        session_id,
        &[
            title_line,
            claude_user_prompt("prompt", timestamp),
            claude_usage_assistant(
                message_id,
                timestamp,
                "preview",
                input,
                cache_create,
                cache_read,
                output,
            ),
        ]
        .join("\n"),
    );
}

#[test]
fn usage_ranking_orders_both_harnesses_by_total_descending() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    // Claude: 10 + 2000 + 30000 + 50 = 32060.
    let claude_id = "aaaaaaaa-1111-2222-3333-444444444444";
    plant_ranking_claude(
        claude.path(),
        workdir.path(),
        claude_id,
        "Claude chat",
        "msg_claude_rank",
        "2026-08-20T04:00:10.000Z",
        10,
        2000,
        30000,
        50,
    );
    // Codex: input 50000 (no cache), output 12000 -> total 62000, bigger.
    let codex_id = "c0de0101-1111-2222-3333-444444444444";
    let tc = codex_token_count(
        "2026-08-28T10:01:00.000Z",
        50000,
        0,
        0,
        12000,
        50000,
        0,
        0,
        12000,
    );
    plant_codex_usage_session(codex.path(), codex_id, workdir.path(), None, &[&tc]);
    plant_codex_title(codex.path(), codex_id, "Codex chat");

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("usage")
        .assert()
        .success()
        // Header columns.
        .stdout(predicates::str::contains("session"))
        .stdout(predicates::str::contains("project"))
        .stdout(predicates::str::contains("harness"))
        .stdout(predicates::str::contains("title"))
        .stdout(predicates::str::contains("subagents"))
        .stdout(predicates::str::contains("calls"))
        .stdout(predicates::str::contains("input"))
        .stdout(predicates::str::contains("cache-write"))
        .stdout(predicates::str::contains("cache-read"))
        .stdout(predicates::str::contains("output"))
        .stdout(predicates::str::contains("total"))
        // Both Harnesses listed with titles and harness names.
        .stdout(predicates::str::contains("claude"))
        .stdout(predicates::str::contains("codex"))
        .stdout(predicates::str::contains("Claude chat"))
        .stdout(predicates::str::contains("Codex chat"))
        .stdout(predicates::str::contains(&claude_id[..8]))
        .stdout(predicates::str::contains(&codex_id[..8]))
        // Thousands separators.
        .stdout(predicates::str::contains("30,000"))
        .stdout(predicates::str::contains("50,000"))
        .stdout(predicates::str::contains("62,000"))
        // Biggest total first: Codex (62000) before Claude (32060).
        .stdout(predicates::function::function(|out: &str| {
            out.find(&codex_id[..8]).unwrap() < out.find(&claude_id[..8]).unwrap()
        }));
}

#[test]
fn usage_ranking_sort_output_input_calls_reorder() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // big: 1 call, input 50000, output 1000 -> total 51000.
    let big_id = "bbbbbbbb-1111-2222-3333-444444444444";
    plant_ranking_claude(
        store.path(),
        workdir.path(),
        big_id,
        "big total",
        "msg_big",
        "2026-08-20T04:00:10.000Z",
        50000,
        0,
        0,
        1000,
    );
    // out: 1 call, input 100, output 40000 -> total 40100.
    let out_id = "cccccccc-1111-2222-3333-444444444444";
    plant_ranking_claude(
        store.path(),
        workdir.path(),
        out_id,
        "high output",
        "msg_out",
        "2026-08-20T04:00:11.000Z",
        100,
        0,
        0,
        40000,
    );
    // many: 5 calls each input 1000 output 100 -> total 5500, calls 5.
    let many_id = "dddddddd-1111-2222-3333-444444444444";
    let mut lines = vec![
        serde_json::json!({"type":"ai-title","aiTitle":"many calls"}).to_string(),
        claude_user_prompt("prompt", "2026-08-20T04:00:12.000Z"),
    ];
    for i in 0..5 {
        lines.push(claude_usage_assistant(
            &format!("msg_many_{i}"),
            "2026-08-20T04:00:13.000Z",
            "preview",
            1000,
            0,
            0,
            100,
        ));
    }
    plant_claude_session(store.path(), workdir.path(), many_id, &lines.join("\n"));

    let ranking = |args: &[&str]| {
        let mut cmd = agsearch_command();
        cmd.current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(store.path())
            .arg("usage");
        for a in args {
            cmd.arg(a);
        }
        cmd.assert().success().get_output().stdout.clone()
    };

    // Default total: big (51000) > out (40100) > many (5500).
    let out = String::from_utf8(ranking(&[])).unwrap();
    assert!(
        out.find(&big_id[..8]).unwrap() < out.find(&out_id[..8]).unwrap()
            && out.find(&out_id[..8]).unwrap() < out.find(&many_id[..8]).unwrap(),
        "default total order wrong:\n{out}"
    );
    // --sort output: out (40000) > big (1000) > many (500).
    let out = String::from_utf8(ranking(&["--sort", "output"])).unwrap();
    assert!(
        out.find(&out_id[..8]).unwrap() < out.find(&big_id[..8]).unwrap()
            && out.find(&big_id[..8]).unwrap() < out.find(&many_id[..8]).unwrap(),
        "--sort output order wrong:\n{out}"
    );
    // --sort input: big (50000) > many (5000) > out (100).
    let out = String::from_utf8(ranking(&["--sort", "input"])).unwrap();
    assert!(
        out.find(&big_id[..8]).unwrap() < out.find(&many_id[..8]).unwrap()
            && out.find(&many_id[..8]).unwrap() < out.find(&out_id[..8]).unwrap(),
        "--sort input order wrong:\n{out}"
    );
    // --sort calls: many (5) > big (1, total 51000) > out (1, total 40100).
    let out = String::from_utf8(ranking(&["--sort", "calls"])).unwrap();
    assert!(
        out.find(&many_id[..8]).unwrap() < out.find(&big_id[..8]).unwrap()
            && out.find(&big_id[..8]).unwrap() < out.find(&out_id[..8]).unwrap(),
        "--sort calls order wrong:\n{out}"
    );
}

#[test]
fn usage_ranking_limit_truncates() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    for (id, total_out) in [
        ("eeeeeee1-1111-2222-3333-444444444444", 50000u64),
        ("eeeeeee2-1111-2222-3333-444444444444", 40000u64),
        ("eeeeeee3-1111-2222-3333-444444444444", 30000u64),
    ] {
        plant_ranking_claude(
            store.path(),
            workdir.path(),
            id,
            "chat",
            &format!("msg_{}", &id[..8]),
            "2026-08-20T04:00:10.000Z",
            100,
            0,
            0,
            total_out,
        );
    }

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg("--limit")
        .arg("2")
        .assert()
        .success()
        .stdout(predicates::str::contains("eeeeeee1"))
        .stdout(predicates::str::contains("eeeeeee2"))
        .stdout(predicates::str::contains("eeeeeee3").not());

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg("--limit")
        .arg("1")
        .assert()
        .success()
        .stdout(predicates::str::contains("eeeeeee1"))
        .stdout(predicates::str::contains("eeeeeee2").not());
}

#[test]
fn usage_ranking_all_and_project_scope_like_sessions() {
    let workdir = tempdir().unwrap();
    let other = tempdir().unwrap();
    let store = tempdir().unwrap();
    let current_id = "f0f0f0f0-1111-2222-3333-444444444444";
    plant_ranking_claude(
        store.path(),
        workdir.path(),
        current_id,
        "current project chat",
        "msg_current",
        "2026-08-20T04:00:10.000Z",
        1000,
        0,
        0,
        100,
    );
    let other_id = "e1e1e1e1-aaaa-bbbb-cccc-444444444444";
    plant_ranking_claude(
        store.path(),
        other.path(),
        other_id,
        "other project chat",
        "msg_other",
        "2026-08-20T04:00:10.000Z",
        2000,
        0,
        0,
        200,
    );

    // Default: only the current directory's Project.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&current_id[..8]))
        .stdout(predicates::str::contains(&other_id[..8]).not());

    // --all: every Project.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg("--all")
        .assert()
        .success()
        .stdout(predicates::str::contains(&current_id[..8]))
        .stdout(predicates::str::contains(&other_id[..8]));

    // --project: substring of the other cwd's encoded key. Tempdir names are
    // random (`-tmpXXXXXX` after encoding), so use the trailing random suffix
    // (no leading dash, distinctive from the current Project).
    let other_name = other
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .replace(['.', '_'], "-");
    let substr = other_name[other_name.len().saturating_sub(6)..].to_string();
    assert!(substr.len() >= 4 && !substr.starts_with('-'));
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg("--project")
        .arg(&substr)
        .assert()
        .success()
        .stdout(predicates::str::contains(&other_id[..8]));
}

#[test]
fn usage_ranking_harness_narrows_like_sessions() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let claude_id = "11112222-1111-2222-3333-444444444444";
    plant_ranking_claude(
        claude.path(),
        workdir.path(),
        claude_id,
        "claude chat",
        "msg_harness_claude",
        "2026-08-20T04:00:10.000Z",
        5000,
        0,
        0,
        500,
    );
    let codex_id = "c0de0202-1111-2222-3333-444444444444";
    let tc = codex_token_count("2026-08-28T10:01:00.000Z", 6000, 0, 0, 600, 6000, 0, 0, 600);
    plant_codex_usage_session(codex.path(), codex_id, workdir.path(), None, &[&tc]);

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("--harness")
        .arg("claude")
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&claude_id[..8]))
        .stdout(predicates::str::contains(&codex_id[..8]).not());

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("--harness")
        .arg("codex")
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&codex_id[..8]))
        .stdout(predicates::str::contains(&claude_id[..8]).not());
}

#[test]
fn usage_ranking_since_filters_like_sessions() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let old_id = "22223333-1111-2222-3333-444444444444";
    plant_ranking_claude(
        store.path(),
        workdir.path(),
        old_id,
        "old chat",
        "msg_old",
        "2026-01-01T00:00:00.000Z",
        9000,
        0,
        0,
        900,
    );
    let new_id = "33334444-1111-2222-3333-444444444444";
    plant_ranking_claude(
        store.path(),
        workdir.path(),
        new_id,
        "new chat",
        "msg_new",
        "2026-08-20T04:00:10.000Z",
        1000,
        0,
        0,
        100,
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg("--since")
        .arg("2026-06-01")
        .assert()
        .success()
        .stdout(predicates::str::contains(&new_id[..8]))
        .stdout(predicates::str::contains(&old_id[..8]).not());
}

#[test]
fn usage_ranking_excludes_current_family_by_default() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let current_id = "44445555-aaaa-bbbb-cccc-ddddeeee0100";
    plant_ranking_claude(
        store.path(),
        workdir.path(),
        current_id,
        "current chat",
        "msg_current_rank",
        "2026-08-20T04:00:10.000Z",
        90000,
        0,
        0,
        9000,
    );
    let earlier_id = "55556666-aaaa-bbbb-cccc-ddddeeee0101";
    plant_ranking_claude(
        store.path(),
        workdir.path(),
        earlier_id,
        "earlier chat",
        "msg_earlier_rank",
        "2026-08-19T04:00:10.000Z",
        1000,
        0,
        0,
        100,
    );

    // Excluded by default even though it is the biggest.
    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", current_id)
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&earlier_id[..8]))
        .stdout(predicates::str::contains(&current_id[..8]).not());

    // --include-current puts it back, biggest first.
    agsearch_command()
        .current_dir(workdir.path())
        .env("CLAUDE_CODE_SESSION_ID", current_id)
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg("--include-current")
        .assert()
        .success()
        .stdout(predicates::str::contains(&earlier_id[..8]))
        .stdout(predicates::str::contains(&current_id[..8]))
        .stdout(predicates::function::function(|out: &str| {
            out.find(current_id.get(..8).unwrap()).unwrap()
                < out.find(earlier_id.get(..8).unwrap()).unwrap()
        }));
}

#[test]
fn usage_ranking_omits_sessions_without_usage_and_reports_skipped() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let used_id = "66667777-1111-2222-3333-444444444444";
    plant_ranking_claude(
        store.path(),
        workdir.path(),
        used_id,
        "used chat",
        "msg_used",
        "2026-08-20T04:00:10.000Z",
        7000,
        0,
        0,
        700,
    );
    // No assistant Records at all: zero calls carrying Usage.
    let unused_id = "77778888-1111-2222-3333-444444444444";
    plant_claude_session(
        store.path(),
        workdir.path(),
        unused_id,
        &[
            serde_json::json!({"type":"ai-title","aiTitle":"unused chat"}).to_string(),
            claude_user_prompt("prompt without usage", "2026-08-20T04:00:10.000Z"),
        ]
        .join("\n"),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&used_id[..8]))
        .stdout(predicates::str::contains(&unused_id[..8]).not())
        .stdout(predicates::str::contains("skipped"))
        .stdout(predicates::str::contains("1 session"))
        .stdout(predicates::str::contains("no usage"));
}

// --- usage: fold subagent threads into the parent Session (issue 33) ---

/// Plant a Claude parent Session plus one worker thread with distinct previews
/// and Usage, for fold fixtures. Returns (parent_id, worker_id).
fn plant_claude_parent_with_worker(
    store: &std::path::Path,
    cwd: &std::path::Path,
    parent_id: &str,
    worker_id: &str,
    parent_preview: &str,
    worker_preview: &str,
) {
    plant_claude_session(
        store,
        cwd,
        parent_id,
        &[
            serde_json::json!({"type":"ai-title","aiTitle":"parent chat"}).to_string(),
            claude_user_prompt("parent prompt", "2026-08-20T04:00:00.000Z"),
            claude_usage_assistant(
                "msg_parent_fold",
                "2026-08-20T04:00:10.000Z",
                parent_preview,
                10,
                2000,
                30000,
                50,
            ),
        ]
        .join("\n"),
    );
    plant_claude_subagent(
        store,
        cwd,
        parent_id,
        worker_id,
        &[
            claude_user_prompt("worker prompt", "2026-08-20T04:00:05.000Z"),
            claude_usage_assistant(
                "msg_worker_fold",
                "2026-08-20T04:00:15.000Z",
                worker_preview,
                7,
                1000,
                20000,
                30,
            ),
        ]
        .join("\n"),
    );
}

#[test]
fn usage_ranking_folds_claude_worker_into_parent() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // Distinct prefixes so parent and worker rows are distinguishable.
    let parent_id = "a1a1a1a1-1111-2222-3333-444444444444";
    let worker_id = "b2b2b2b2-1111-2222-3333-444444444444";
    plant_claude_parent_with_worker(
        store.path(),
        workdir.path(),
        parent_id,
        worker_id,
        "claude parent fold marker",
        "claude worker fold marker",
    );

    // Parent 10+2000+30000+50=32,060; worker 7+1000+20000+30=21,037;
    // folded 17+3000+50000+80=53,097, 2 calls, 1 subagent.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&parent_id[..8]))
        .stdout(predicates::str::contains(&worker_id[..8]).not())
        .stdout(predicates::str::contains("53,097"))
        .stdout(predicates::str::contains("subagents"))
        // The folded row carries subagents=1 and calls=2: find the parent's
        // line and check it shows both alongside the folded total.
        .stdout(predicates::function::function(|out: &str| {
            out.lines().any(|line| {
                line.contains(&parent_id[..8]) && line.contains("53,097") && line.contains("2")
            })
        }));
}

#[test]
fn usage_ranking_include_subagents_lists_claude_workers_separately() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let parent_id = "a3a3a3a3-1111-2222-3333-444444444444";
    let worker_id = "b4b4b4b4-1111-2222-3333-444444444444";
    plant_claude_parent_with_worker(
        store.path(),
        workdir.path(),
        parent_id,
        worker_id,
        "claude parent separate marker",
        "claude worker separate marker",
    );

    // With --include-subagents there is no folding: both rows appear with
    // their own totals (32,060 and 21,037), never the folded 53,097.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("--include-subagents")
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&parent_id[..8]))
        .stdout(predicates::str::contains(&worker_id[..8]))
        .stdout(predicates::str::contains("32,060"))
        .stdout(predicates::str::contains("21,037"))
        .stdout(predicates::str::contains("53,097").not());
}

#[test]
fn usage_breakdown_claude_parent_includes_marked_worker_and_matches_ranking() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let parent_id = "a5a5a5a5-1111-2222-3333-444444444444";
    let worker_id = "b6b6b6b6-1111-2222-3333-444444444444";
    plant_claude_parent_with_worker(
        store.path(),
        workdir.path(),
        parent_id,
        worker_id,
        "claude parent breakdown marker",
        "claude worker breakdown marker",
    );

    // Breakdown of the parent includes both calls, the worker row marked with
    // its short id in the subagent column, and the family total.
    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg(&parent_id[..8])
        .assert()
        .success()
        .stdout(predicates::str::contains("claude parent breakdown marker"))
        .stdout(predicates::str::contains("claude worker breakdown marker"))
        .stdout(predicates::str::contains("subagent"))
        .stdout(predicates::str::contains(&worker_id[..8]))
        .stdout(predicates::str::contains("53,097"))
        .stdout(predicates::str::contains("2 calls"))
        // The worker's line carries its subagent id; the parent's line does not.
        .stdout(predicates::function::function(|out: &str| {
            let worker_line = out
                .lines()
                .find(|line| line.contains("claude worker breakdown marker"));
            let parent_line = out
                .lines()
                .find(|line| line.contains("claude parent breakdown marker"));
            match (worker_line, parent_line) {
                (Some(w), Some(p)) => w.contains(&worker_id[..8]) && !p.contains(&worker_id[..8]),
                _ => false,
            }
        }));

    // The breakdown total matches the ranking row's folded total.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains("53,097"));
}

#[test]
fn usage_breakdown_resolves_a_claude_worker_id_directly() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    let parent_id = "a7a7a7a7-1111-2222-3333-444444444444";
    let worker_id = "b8b8b8b8-1111-2222-3333-444444444444";
    plant_claude_parent_with_worker(
        store.path(),
        workdir.path(),
        parent_id,
        worker_id,
        "claude parent handoff marker",
        "claude worker handoff marker",
    );

    // A worker row from `usage --include-subagents` hands off to its own
    // breakdown, even though search/show keep hiding Claude workers.
    // Worker total: 7+1000+20000+30=21,037, 1 call.
    for extra in [vec!["--include-subagents"], vec![]] {
        let mut cmd = agsearch_command();
        cmd.current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(store.path());
        for flag in extra {
            cmd.arg(flag);
        }
        cmd.arg("usage").arg(&worker_id[..8]);
        cmd.assert()
            .success()
            .stdout(predicates::str::contains("claude worker handoff marker"))
            .stdout(predicates::str::contains("21,037"))
            .stdout(predicates::str::contains("1 call"));
    }
}

#[test]
fn usage_breakdown_codex_malformed_final_disables_the_check() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let session_id = "c0de0009-1111-2222-3333-444444444444";
    // One valid call, but the final total is not an object: no usable counts,
    // so the mismatch check disables itself instead of warning on zeros.
    let malformed = serde_json::json!({
        "timestamp": "2026-08-28T10:01:00.000Z",
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "last_token_usage": {
                    "input_tokens": 12345,
                    "cache_write_input_tokens": 500,
                    "cached_input_tokens": 2000,
                    "output_tokens": 50,
                    "reasoning_output_tokens": 0,
                    "total_tokens": 12395
                },
                "total_token_usage": "not-an-object",
                "model_context_window": null
            },
            "rate_limits": null
        }
    })
    .to_string();
    plant_codex_usage_session(
        codex.path(),
        session_id,
        workdir.path(),
        Some("codex-fable-7"),
        &[&malformed],
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("usage")
        .arg(&session_id[..8])
        .assert()
        .success()
        .stderr(predicates::str::contains("warning").not())
        .stdout(predicates::str::contains("12,395"))
        .stdout(predicates::str::contains("1 call"));
}

#[test]
fn usage_ranking_folds_codex_worker_into_parent() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let parent_id = "c0de1111-0000-0000-0000-000000000000";
    let worker_id = "c0de2222-0000-0000-0000-000000000000";
    // Parent: raw 10000-2000-500=7500 input, total 7500+500+2000+100=10,100.
    let parent_tc = codex_token_count(
        "2026-08-28T10:01:00.000Z",
        10000,
        500,
        2000,
        100,
        10000,
        500,
        2000,
        100,
    );
    // Worker: raw 5000-1000-200=3800 input, total 3800+200+1000+50=5,050.
    let worker_tc = codex_token_count(
        "2026-08-28T10:02:00.000Z",
        5000,
        200,
        1000,
        50,
        5000,
        200,
        1000,
        50,
    );
    plant_codex_usage_session(
        codex.path(),
        parent_id,
        workdir.path(),
        Some("codex-fable-7"),
        &[&parent_tc],
    );
    plant_codex_title(codex.path(), parent_id, "Codex parent chat");
    // Worker rollout with parent link.
    {
        let meta = serde_json::json!({
            "timestamp": "2026-08-28T10:00:00.000Z",
            "type": "session_meta",
            "payload": {
                "id": worker_id,
                "cwd": workdir.path().to_string_lossy(),
                "thread_source": "subagent",
                "parent_thread_id": parent_id,
                "model": "codex-fable-7",
                "agent_nickname": "fixture-worker",
                "source": {"subagent": {"thread_spawn": {"agent_nickname": "nested-worker", "parent_thread_id": parent_id}}}
            }
        });
        write_codex_rollout(codex.path(), worker_id, meta, &[&worker_tc]);
    }
    // Folded: input 11,300, cache-write 700, cache-read 3,000, output 150,
    // total 15,150, 2 calls, 1 subagent.

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&parent_id[..8]))
        .stdout(predicates::str::contains(&worker_id[..8]).not())
        .stdout(predicates::str::contains("15,150"))
        .stdout(predicates::str::contains("11,300"));
}

#[test]
fn usage_ranking_include_subagents_lists_codex_workers_separately() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let parent_id = "c0de3333-0000-0000-0000-000000000000";
    let worker_id = "c0de4444-0000-0000-0000-000000000000";
    let parent_tc = codex_token_count(
        "2026-08-28T10:01:00.000Z",
        10000,
        500,
        2000,
        100,
        10000,
        500,
        2000,
        100,
    );
    let worker_tc = codex_token_count(
        "2026-08-28T10:02:00.000Z",
        5000,
        200,
        1000,
        50,
        5000,
        200,
        1000,
        50,
    );
    plant_codex_usage_session(
        codex.path(),
        parent_id,
        workdir.path(),
        Some("codex-fable-7"),
        &[&parent_tc],
    );
    {
        let meta = serde_json::json!({
            "timestamp": "2026-08-28T10:00:00.000Z",
            "type": "session_meta",
            "payload": {
                "id": worker_id,
                "cwd": workdir.path().to_string_lossy(),
                "thread_source": "subagent",
                "parent_thread_id": parent_id,
                "model": "codex-fable-7",
                "agent_nickname": "fixture-worker",
                "source": {"subagent": {"thread_spawn": {"agent_nickname": "nested-worker", "parent_thread_id": parent_id}}}
            }
        });
        write_codex_rollout(codex.path(), worker_id, meta, &[&worker_tc]);
    }

    // No folding: both rows with own totals (10,100 and 5,050), never 15,150.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("--include-subagents")
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&parent_id[..8]))
        .stdout(predicates::str::contains(&worker_id[..8]))
        .stdout(predicates::str::contains("10,100"))
        .stdout(predicates::str::contains("5,050"))
        .stdout(predicates::str::contains("15,150").not());
}

#[test]
fn usage_breakdown_codex_parent_includes_marked_worker_and_matches_ranking() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    let parent_id = "c0de5555-0000-0000-0000-000000000000";
    let worker_id = "c0de6666-0000-0000-0000-000000000000";
    let parent_tc = codex_token_count(
        "2026-08-28T10:01:00.000Z",
        10000,
        500,
        2000,
        100,
        10000,
        500,
        2000,
        100,
    );
    let worker_tc = codex_token_count(
        "2026-08-28T10:02:00.000Z",
        5000,
        200,
        1000,
        50,
        5000,
        200,
        1000,
        50,
    );
    plant_codex_usage_session(
        codex.path(),
        parent_id,
        workdir.path(),
        Some("codex-fable-7"),
        &[&parent_tc],
    );
    {
        let meta = serde_json::json!({
            "timestamp": "2026-08-28T10:00:00.000Z",
            "type": "session_meta",
            "payload": {
                "id": worker_id,
                "cwd": workdir.path().to_string_lossy(),
                "thread_source": "subagent",
                "parent_thread_id": parent_id,
                "model": "codex-fable-7",
                "agent_nickname": "fixture-worker",
                "source": {"subagent": {"thread_spawn": {"agent_nickname": "nested-worker", "parent_thread_id": parent_id}}}
            }
        });
        write_codex_rollout(codex.path(), worker_id, meta, &[&worker_tc]);
    }

    // Breakdown shows both token_counts (2 calls), the worker row marked with
    // its short id in the subagent column (preview is empty for Codex), and
    // the family total 15,150.
    agsearch_command()
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("usage")
        .arg(&parent_id[..8])
        .assert()
        .success()
        .stdout(predicates::str::contains("subagent"))
        .stdout(predicates::str::contains(&worker_id[..8]))
        .stdout(predicates::str::contains("15,150"))
        .stdout(predicates::str::contains("11,300"))
        .stdout(predicates::str::contains("2 calls"));

    // Ranking row matches the breakdown total.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains("15,150"));
}

#[test]
fn usage_ranking_folds_transitive_claude_grandchild_into_root() {
    let workdir = tempdir().unwrap();
    let store = tempdir().unwrap();
    // Three-level family: parent -> worker -> grandchild. Every descendant's
    // Usage folds into the root (spec: "every descendant thread").
    let parent_id = "c1c1c1c1-1111-2222-3333-444444444444";
    let worker_id = "d2d2d2d2-1111-2222-3333-444444444444";
    let grandchild_id = "e3e3e3e3-1111-2222-3333-444444444444";
    plant_claude_parent_with_worker(
        store.path(),
        workdir.path(),
        parent_id,
        worker_id,
        "transitive parent marker",
        "transitive worker marker",
    );
    // Grandchild nested under the worker: <parent>/subagents/<worker>/subagents/.
    // File and dir share the worker stem (<worker>.jsonl vs <worker>/) so both
    // coexist; `claude_parent_id` reads the grandparent dir name as the parent.
    let encoded = agsearch::encode_project_dir(&workdir.path().to_string_lossy());
    let grandchild_dir = store
        .path()
        .join("projects")
        .join(encoded)
        .join(parent_id)
        .join("subagents")
        .join(worker_id)
        .join("subagents");
    std::fs::create_dir_all(&grandchild_dir).unwrap();
    std::fs::write(
        grandchild_dir.join(format!("{grandchild_id}.jsonl")),
        [
            claude_user_prompt("grandchild prompt", "2026-08-20T04:00:07.000Z"),
            claude_usage_assistant(
                "msg_grandchild_fold",
                "2026-08-20T04:00:18.000Z",
                "transitive grandchild marker",
                3,
                500,
                10000,
                20,
            ),
        ]
        .join("\n"),
    )
    .unwrap();

    // Parent 32,060 + worker 21,037 + grandchild 10,523 = 63,620, 3 calls,
    // 2 subagents. Only the root lists; workers never appear as own rows.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .assert()
        .success()
        .stdout(predicates::str::contains(&parent_id[..8]))
        .stdout(predicates::str::contains(&worker_id[..8]).not())
        .stdout(predicates::str::contains(&grandchild_id[..8]).not())
        .stdout(predicates::str::contains("63,620"))
        .stdout(predicates::function::function(|out: &str| {
            out.lines()
                .any(|line| line.contains(&parent_id[..8]) && line.contains("63,620"))
        }));

    // Breakdown of the root includes all three, both workers marked, total matches.
    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("usage")
        .arg(&parent_id[..8])
        .assert()
        .success()
        .stdout(predicates::str::contains("transitive parent marker"))
        .stdout(predicates::str::contains("transitive worker marker"))
        .stdout(predicates::str::contains("transitive grandchild marker"))
        .stdout(predicates::str::contains(&worker_id[..8]))
        .stdout(predicates::str::contains(&grandchild_id[..8]))
        .stdout(predicates::str::contains("63,620"))
        .stdout(predicates::str::contains("3 calls"));
}

// --- short session-id: shortest unique prefix (ADR 0017) ----------------

const SHARED_A: &str = "abcdefghij1-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const SHARED_B: &str = "abcdefghij2-bbbb-bbbb-bbbb-bbbbbbbbbbbb";

fn prompt_line(text: &str) -> String {
    serde_json::json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
}

#[test]
fn sessions_that_share_10_characters_show_distinct_short_ids_that_open_them() {
    let store = tempdir().unwrap();
    plant_session(
        store.path(),
        "E--projects-demo",
        SHARED_A,
        &prompt_line("alpha prompt"),
    );
    plant_session(
        store.path(),
        "E--projects-demo",
        SHARED_B,
        &prompt_line("beta prompt"),
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .args(["sessions", "--all"])
        .assert()
        .success()
        .stdout(predicates::str::contains("claude · abcdefghij1 ·"))
        .stdout(predicates::str::contains("claude · abcdefghij2 ·"));

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .args(["search", "--all", "prompt"])
        .assert()
        .success()
        .stdout(predicates::str::contains("claude · abcdefghij1 ·"))
        .stdout(predicates::str::contains("claude · abcdefghij2 ·"));

    for (short, text) in [
        ("abcdefghij1", "alpha prompt"),
        ("abcdefghij2", "beta prompt"),
    ] {
        agsearch_command()
            .arg("--claude-dir")
            .arg(store.path())
            .args(["show", short])
            .assert()
            .success()
            .stdout(predicates::str::contains(text));
    }
}

#[test]
fn the_more_hint_and_failed_header_print_the_unique_short_id() {
    let store = tempdir().unwrap();
    let lines = [
        prompt_line("needle one"),
        prompt_line("needle two"),
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#.to_string(),
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"error: boom"}]}}"#.to_string(),
    ]
    .join("\n");
    plant_session(store.path(), "E--projects-demo", SHARED_A, &lines);
    plant_session(
        store.path(),
        "E--projects-demo",
        SHARED_B,
        &prompt_line("other"),
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .args(["search", "--all", "--max-per-session", "1", "needle"])
        .assert()
        .success()
        .stdout(predicates::str::contains("agsearch show abcdefghij1\n"));

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .args(["search", "--all", "--failed"])
        .assert()
        .success()
        .stdout(predicates::str::contains("claude · abcdefghij1 ·"));
}

#[test]
fn claude_and_codex_ids_with_distinct_first_8_characters_show_8() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "11111111-2222-3333-4444-555555555555",
        &prompt_line("claude prompt"),
    );
    plant_codex_session(
        codex.path(),
        "22222222-2222-3333-4444-555555555555",
        workdir.path(),
        "user",
        &[],
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains("claude · 11111111 ·"))
        .stdout(predicates::str::contains("codex · 22222222 ·"));
}

#[test]
fn an_ambiguous_prefix_still_reports_the_candidates() {
    let store = tempdir().unwrap();
    plant_session(
        store.path(),
        "E--projects-demo",
        SHARED_A,
        &prompt_line("a"),
    );
    plant_session(
        store.path(),
        "E--projects-demo",
        SHARED_B,
        &prompt_line("b"),
    );

    agsearch_command()
        .arg("--claude-dir")
        .arg(store.path())
        .args(["show", "abcdefghij"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("ambiguous"))
        .stderr(predicates::str::contains(SHARED_A))
        .stderr(predicates::str::contains(SHARED_B));
}

#[test]
fn short_ids_stay_unique_against_sessions_outside_the_scope() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let codex = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        SHARED_A,
        &prompt_line("in scope"),
    );
    plant_session(
        claude.path(),
        "E--projects-elsewhere",
        "abcdefghij3-cccc",
        &prompt_line("x"),
    );
    plant_codex_session(codex.path(), SHARED_B, workdir.path(), "user", &[]);

    // The other Project and the unselected Codex Store still count.
    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--codex-dir")
        .arg(codex.path())
        .args(["--harness", "claude", "sessions"])
        .assert()
        .success()
        .stdout(predicates::str::contains("claude · abcdefghij1 ·"));
}

/// The `session`, `message`, and `part` tables as OpenCode 1.18 creates them.
const OPENCODE_SCHEMA: &str = "
CREATE TABLE `session` (
  `id` text PRIMARY KEY,
  `project_id` text NOT NULL,
  `workspace_id` text,
  `parent_id` text,
  `slug` text NOT NULL,
  `directory` text NOT NULL,
  `path` text,
  `title` text NOT NULL,
  `version` text NOT NULL,
  `share_url` text,
  `summary_additions` integer,
  `summary_deletions` integer,
  `summary_files` integer,
  `summary_diffs` text,
  `metadata` text,
  `cost` real DEFAULT 0 NOT NULL,
  `tokens_input` integer DEFAULT 0 NOT NULL,
  `tokens_output` integer DEFAULT 0 NOT NULL,
  `tokens_reasoning` integer DEFAULT 0 NOT NULL,
  `tokens_cache_read` integer DEFAULT 0 NOT NULL,
  `tokens_cache_write` integer DEFAULT 0 NOT NULL,
  `revert` text,
  `permission` text,
  `agent` text,
  `model` text,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL,
  `time_compacting` integer,
  `time_archived` integer
);
CREATE TABLE `message` (
  `id` text PRIMARY KEY,
  `session_id` text NOT NULL,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL,
  `data` text NOT NULL
);
CREATE TABLE `part` (
  `id` text PRIMARY KEY,
  `message_id` text NOT NULL,
  `session_id` text NOT NULL,
  `time_created` integer NOT NULL,
  `time_updated` integer NOT NULL,
  `data` text NOT NULL
);
";

/// 2026-08-28T10:00:00Z in epoch milliseconds, the unit OpenCode stores.
const OPENCODE_AUG_28: i64 = 1_787_911_200_000;
/// 2026-01-01T10:00:00Z in epoch milliseconds.
const OPENCODE_JAN_1: i64 = 1_767_261_600_000;

const OPENCODE_A: &str = "ses_f36c0fcffffeYOE6LLg4PRibgr";
const OPENCODE_B: &str = "ses_f36c0fcf1ffeZevQ12apJFoNBL";
const OPENCODE_CHILD: &str = "ses_a1b2c3d4effe01IgDuAmyIFsyA";

/// One `session` row for [`plant_opencode_session`].
struct OpenCodeSession<'a> {
    id: &'a str,
    parent_id: Option<&'a str>,
    directory: String,
    title: &'a str,
    time_updated: i64,
    archived: bool,
}

impl<'a> OpenCodeSession<'a> {
    fn new(id: &'a str, directory: &std::path::Path, title: &'a str) -> Self {
        Self {
            id,
            parent_id: None,
            // OpenCode records Windows directories with forward slashes.
            directory: directory.to_string_lossy().replace('\\', "/"),
            title,
            time_updated: OPENCODE_AUG_28,
            archived: false,
        }
    }
}

/// Add a Session row to the `opencode.db` in `store`, creating the database
/// from [`OPENCODE_SCHEMA`] on first use.
fn plant_opencode_session(store: &std::path::Path, session: OpenCodeSession) {
    let path = store.join("opencode.db");
    let fresh = !path.exists();
    let db = rusqlite::Connection::open(&path).unwrap();
    if fresh {
        db.execute_batch(OPENCODE_SCHEMA).unwrap();
    }
    db.execute(
        "INSERT INTO session (id, project_id, parent_id, slug, directory, title, version,
            time_created, time_updated, time_archived)
         VALUES (?1, 'prj_fixture', ?2, 'fixture-slug', ?3, ?4, '1.18.32', ?5, ?5, ?6)",
        rusqlite::params![
            session.id,
            session.parent_id,
            session.directory,
            session.title,
            session.time_updated,
            session.archived.then_some(session.time_updated),
        ],
    )
    .unwrap();
}

/// A command with only the given OpenCode Store configured.
fn agsearch_with_opencode(workdir: &std::path::Path, opencode: &std::path::Path) -> Command {
    let mut command = agsearch_command();
    command
        .current_dir(workdir)
        .arg("--claude-dir")
        .arg(workdir.join("missing-claude"))
        .arg("--opencode-dir")
        .arg(opencode);
    command
}

#[test]
fn sessions_lists_opencode_sessions_with_their_identity() {
    let workdir = tempdir().unwrap();
    let opencode = tempdir().unwrap();
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession::new(OPENCODE_A, workdir.path(), "Fix the parser"),
    );
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession {
            time_updated: OPENCODE_JAN_1,
            ..OpenCodeSession::new(
                OPENCODE_B,
                workdir.path(),
                "New session - 2026-01-01T10:00:00.000Z",
            )
        },
    );

    let directory = workdir.path().to_string_lossy().replace('\\', "/");
    agsearch_with_opencode(workdir.path(), opencode.path())
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "opencode · f36c0fcff · {directory} · Fix the parser · 2026-08-28\n"
        )))
        .stdout(predicates::str::contains(format!(
            "opencode · f36c0fcf1 · {directory} · (untitled) · 2026-01-01\n"
        )))
        .stdout(predicates::str::contains("ses_").not());
}

#[test]
fn projects_merge_opencode_with_claude_code() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let opencode = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "11111111-2222-3333-4444-555555555555",
        &prompt_line("claude side"),
    );
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession::new(OPENCODE_A, workdir.path(), "OpenCode side"),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(claude.path())
        .arg("--opencode-dir")
        .arg(opencode.path())
        .arg("projects")
        .assert()
        .success()
        .stdout(predicates::str::contains(" · 2 sessions · "))
        .stdout(predicates::str::contains(" · 1 session · ").not());
}

#[test]
fn opencode_child_sessions_are_opt_in_and_archived_sessions_are_listed() {
    let workdir = tempdir().unwrap();
    let opencode = tempdir().unwrap();
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession {
            archived: true,
            ..OpenCodeSession::new(OPENCODE_A, workdir.path(), "Archived parent")
        },
    );
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession {
            parent_id: Some(OPENCODE_A),
            ..OpenCodeSession::new(OPENCODE_CHILD, workdir.path(), "Review (@general subagent)")
        },
    );

    agsearch_with_opencode(workdir.path(), opencode.path())
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains("Archived parent"))
        .stdout(predicates::str::contains("a1b2c3d4").not());
    agsearch_with_opencode(workdir.path(), opencode.path())
        .args(["sessions", "--include-subagents"])
        .assert()
        .success()
        .stdout(predicates::str::contains("opencode · a1b2c3d4 · "));
}

#[test]
fn harness_project_and_since_filter_opencode_sessions() {
    let workdir = tempdir().unwrap();
    let claude = tempdir().unwrap();
    let opencode = tempdir().unwrap();
    plant_claude_session(
        claude.path(),
        workdir.path(),
        "11111111-2222-3333-4444-555555555555",
        &prompt_line("claude side"),
    );
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession::new(OPENCODE_A, workdir.path(), "Recent work"),
    );
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession {
            time_updated: OPENCODE_JAN_1,
            ..OpenCodeSession::new(OPENCODE_B, workdir.path(), "Old work")
        },
    );
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession::new(
            OPENCODE_CHILD,
            std::path::Path::new("/elsewhere/other-repo"),
            "Other repo",
        ),
    );
    let command = || {
        let mut command = agsearch_command();
        command
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(claude.path())
            .arg("--opencode-dir")
            .arg(opencode.path());
        command
    };

    command()
        .args(["--harness", "opencode", "sessions", "--all"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Recent work"))
        .stdout(predicates::str::contains("Other repo"))
        .stdout(predicates::str::contains("claude ·").not());
    command()
        .args(["sessions", "--project", "other-repo"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Other repo"))
        .stdout(predicates::str::contains("Recent work").not());
    command()
        .args(["--harness", "opencode", "sessions", "--since", "2026-08-01"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Recent work"))
        .stdout(predicates::str::contains("Old work").not());
}

#[test]
fn an_opencode_dir_without_a_database_is_a_silent_missing_store() {
    let workdir = tempdir().unwrap();
    let opencode = tempdir().unwrap();

    agsearch_with_opencode(workdir.path(), opencode.path())
        .args(["sessions", "--all"])
        .assert()
        .success()
        .stderr("")
        .stdout("No sessions.\n");
}

#[test]
fn the_default_opencode_store_follows_xdg_data_home() {
    let workdir = tempdir().unwrap();
    let data = tempdir().unwrap();
    let opencode = data.path().join("opencode");
    fs::create_dir_all(&opencode).unwrap();
    plant_opencode_session(
        &opencode,
        OpenCodeSession::new(OPENCODE_A, workdir.path(), "From XDG"),
    );

    agsearch_command()
        .current_dir(workdir.path())
        .env("XDG_DATA_HOME", data.path())
        .arg("--claude-dir")
        .arg(workdir.path().join("missing-claude"))
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains("From XDG"));
}

#[test]
fn an_opencode_id_resolves_with_or_without_its_ses_prefix() {
    let workdir = tempdir().unwrap();
    let opencode = tempdir().unwrap();
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession::new(OPENCODE_A, workdir.path(), "First"),
    );
    plant_opencode_session(
        opencode.path(),
        OpenCodeSession::new(OPENCODE_B, workdir.path(), "Second"),
    );

    for selector in ["f36c0fcff", "ses_f36c0fcff", OPENCODE_A] {
        agsearch_with_opencode(workdir.path(), opencode.path())
            .args(["show", selector])
            .assert()
            .success();
    }
    agsearch_with_opencode(workdir.path(), opencode.path())
        .args(["show", "f36c0fcf"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("ambiguous"));
}
