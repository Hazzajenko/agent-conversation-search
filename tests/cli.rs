//! End-to-end tests driving the `agsearch` binary as a user would.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

fn agsearch_command() -> Command {
    let mut command = Command::cargo_bin("agsearch").unwrap();
    command.env("CODEX_HOME", "__agsearch_test_missing_codex_store__");
    command
}

/// Build a fixture Store under `store_root` containing one Session for the given
/// working directory, then return a `Command` ready to run from that cwd with
/// `--claude-dir` pointed at the fixture.
fn agsearch_in(cwd: &std::path::Path, store_root: &std::path::Path, session_lines: &str) -> Command {
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

fn plant_codex_session(
    codex_dir: &std::path::Path,
    id: &str,
    cwd: &std::path::Path,
    thread_source: &str,
    records: &[&str],
) -> std::path::PathBuf {
    let dir = codex_dir.join("sessions").join("2026").join("08").join("28");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("rollout-2026-08-28T10-00-00-{id}.jsonl"));
    let meta = serde_json::json!({
        "timestamp": "2026-08-28T10:00:00.000Z",
        "type": "session_meta",
        "payload": {
            "id": id,
            "cwd": cwd.to_string_lossy(),
            "thread_source": thread_source,
            "source": if thread_source == "subagent" {
                serde_json::json!({"subagent": {"other": "fixture-worker"}})
            } else {
                serde_json::json!("cli")
            }
        }
    });
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
        &[r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"shared needle from Codex"}]}}"#],
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
        &[r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"subagent marker"}]}}"#],
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
        &[r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"worker-only marker"}]}}"#],
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
        &[r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"environment marker"}]}}"#],
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
        .stdout(predicates::str::contains("[5] assistant: handoff reply marker"));

    command()
        .arg("show")
        .arg("c0de0010")
        .arg("--around")
        .arg("5")
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
            r#"{"type":"response_item","payload":{"type":"custom_tool_call","call_id":"call-1","name":"exec_command","input":"{\"cmd\":\"tool-call-needle\"}"}}"#,
            r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call-1","output":"tool-output-needle"}}"#,
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
        &[r#"{"timestamp":"2026-08-28T10:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"new"}]}}"#],
    );
    let old_path = plant_codex_session(
        codex.path(),
        untitled,
        workdir.path(),
        "user",
        &[r#"{"timestamp":"2026-01-01T10:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"old"}]}}"#],
    );
    let old_text = fs::read_to_string(&old_path)
        .unwrap()
        .replacen("2026-08-28T10:00:00.000Z", "2026-01-01T09:00:00.000Z", 1);
    fs::write(old_path, old_text).unwrap();
    plant_codex_session(
        codex.path(),
        "c0de0014-0000-0000-0000-000000000000",
        workdir.path(),
        "subagent",
        &[r#"{"timestamp":"2026-08-28T11:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"worker"}]}}"#],
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
        &[r#"{"timestamp":"2026-08-28T12:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"codex project marker"}]}}"#],
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
        .stdout(predicates::str::contains("exit 101 · error[E0433]: failed to resolve"));
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
        .join(agsearch::encode_project_dir(&workdir.path().to_string_lossy()));
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
    cmd.arg("").assert().failure().stderr(predicates::str::contains("query is required"));
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
    cmd.arg("z").assert().success().stdout(predicates::str::contains("zebra"));
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
    cmd.arg(" ").assert().success().stdout(predicates::str::contains("one two"));
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
    cmd.arg("").arg("--failed").assert().success().stdout(predicates::str::contains("✗ Bash"));
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
        .map(|i| format!(r#"{{"type":"user","message":{{"role":"user","content":"prompt number {i}"}}}}"#))
        .collect::<Vec<_>>()
        .join("\n");
    let id = plant_session(store.path(), "E--projects-demo", "feed0001-0000-0000-0000-000000000000", &lines);

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
        .map(|i| format!(r#"{{"type":"user","message":{{"role":"user","content":"prompt number {i}"}}}}"#))
        .collect::<Vec<_>>()
        .join("\n");
    let id = plant_session(store.path(), "E--projects-demo", "feed0002-0000-0000-0000-000000000000", &lines);

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
        .join(agsearch::encode_project_dir(&workdir.path().to_string_lossy()));
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
        .join(agsearch::encode_project_dir(&workdir.path().to_string_lossy()));
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
        .join(agsearch::encode_project_dir(&workdir.path().to_string_lossy()));
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
        .stdout(predicates::str::contains("eeee5555-0000-0000-0000-000000000000.jsonl"))
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
        .stdout(predicates::str::contains("E:\\projects\\recent · 2 sessions · 2026-06-02"))
        .stdout(predicates::str::contains("E:\\projects\\ancient · 1 session · 2026-01-01"))
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
