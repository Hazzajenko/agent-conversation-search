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

#[test]
fn search_output_carries_the_session_id_and_turn_handoff() {
    let workdir = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let projects = store.path().join("projects");
    let encoded = ccsearch::encode_project_dir(&workdir.path().to_string_lossy());
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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
    let mut cmd = ccsearch_in(
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
    let mut cmd = ccsearch_in(
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
    let mut cmd = ccsearch_in(
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
        .join(ccsearch::encode_project_dir(&workdir.path().to_string_lossy()));
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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
    let mut cmd = ccsearch_in(
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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
    Command::cargo_bin("ccsearch")
        .unwrap()
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
    let mut cmd = ccsearch_in(
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
    let mut cmd = ccsearch_in(
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
    let mut cmd = ccsearch_in(
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
    let mut cmd = ccsearch_in(
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
    Command::cargo_bin("ccsearch")
        .unwrap()
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
    let mut cmd = ccsearch_in(
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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
fn show_around_windows_the_transcript_on_a_turn() {
    let store = tempfile::tempdir().unwrap();
    // Six user prompts → turns 1..=6, each with a unique marker.
    let lines: String = (1..=6)
        .map(|i| format!(r#"{{"type":"user","message":{{"role":"user","content":"prompt number {i}"}}}}"#))
        .collect::<Vec<_>>()
        .join("\n");
    let id = plant_session(store.path(), "E--projects-demo", "feed0001-0000-0000-0000-000000000000", &lines);

    Command::cargo_bin("ccsearch")
        .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
        .arg("--claude-dir")
        .arg(store.path())
        .arg("show")
        .arg(&id[..8])
        .assert()
        .success()
        .stdout(predicates::str::contains("[thinking:"))
        .stdout(predicates::str::contains("a secret rumination").not());

    Command::cargo_bin("ccsearch")
        .unwrap()
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
        .join(ccsearch::encode_project_dir(&workdir.path().to_string_lossy()));
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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
        .join(ccsearch::encode_project_dir(&workdir.path().to_string_lossy()));
    fs::create_dir_all(&project).unwrap();
    // A Session with no ai-title Record and no Message at all (only a noise
    // Record). search would never surface it; `sessions` must still list it.
    fs::write(
        project.join("dddd4444-0000-0000-0000-000000000000.jsonl"),
        r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-04-01T10:00:00.000Z"}"#,
    )
    .unwrap();

    Command::cargo_bin("ccsearch")
        .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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
    Command::cargo_bin("ccsearch")
        .unwrap()
        .current_dir(workdir.path())
        .arg("--claude-dir")
        .arg(store.path())
        .arg("sessions")
        .assert()
        .success()
        .stdout(predicates::str::contains("No sessions."));

    // --all sees both; --since 2026-05-01 keeps only the recent one.
    Command::cargo_bin("ccsearch")
        .unwrap()
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
        .join(ccsearch::encode_project_dir(&workdir.path().to_string_lossy()));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("eeee5555-0000-0000-0000-000000000000.jsonl"),
        r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-06-01T10:00:00.000Z"}"#,
    )
    .unwrap();

    Command::cargo_bin("ccsearch")
        .unwrap()
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
        Command::cargo_bin("ccsearch")
            .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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
    Command::cargo_bin("ccsearch")
        .unwrap()
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
    Command::cargo_bin("ccsearch")
        .unwrap()
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

    Command::cargo_bin("ccsearch")
        .unwrap()
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
        Command::cargo_bin("ccsearch")
            .unwrap()
            .current_dir(workdir.path())
            .arg("--claude-dir")
            .arg(store.path())
            .arg("projects")
            .arg(flag)
            .assert()
            .failure(); // clap rejects an argument `projects` does not define
    }
}
