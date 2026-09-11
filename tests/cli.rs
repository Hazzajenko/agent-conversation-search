//! End-to-end tests driving the `agsearch` binary as a user would.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

fn agsearch_command() -> Command {
    let mut command = Command::cargo_bin("agsearch").unwrap();
    command.env("CODEX_HOME", "__agsearch_test_missing_codex_store__");
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap(); // cwd has no Project of its own
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let store = tempfile::tempdir().unwrap();
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
    let store = tempfile::tempdir().unwrap();
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
    let store = tempfile::tempdir().unwrap();
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
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
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
    let store = tempfile::tempdir().unwrap();
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
    let store = tempfile::tempdir().unwrap();
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
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap(); // cwd has no Project in the Store
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains(&current_id[..8]));
}

#[test]
fn projects_still_counts_the_current_session() {
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let current_id = plant_current_and_earlier_session(workdir.path(), claude.path());

    agsearch_as_current(workdir.path(), claude.path(), &current_id)
        .arg("projects")
        .assert()
        .success()
        .stdout(predicates::str::contains("2 sessions"));
}

#[test]
fn analysis_outside_a_harness_keeps_every_session() {
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
    let codex = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let claude = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let _ = agsearch_in(
        workdir.path(),
        store.path(),
        &touch_use("t1", "Read", r#"{"file_path":"x.md"}"#),
    );

    for selector in ["", "   ", "/", "docs/"] {
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
fn file_reports_the_standard_empty_message_when_nothing_touches() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
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
