//! `ccsearch` — search your local Claude Code conversation history.

use std::path::{Path, PathBuf};

/// Encode a working-directory path into the directory name Claude Code uses
/// for that Project under `<store>/projects/`.
///
/// Claude Code replaces every character that is not an ASCII letter or digit
/// with `-`. The transform is lossy (distinct paths can collapse to the same
/// name) and is therefore one-way — see ADR-0001.
pub fn encode_project_dir(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Find the Project directories under `projects_root` that correspond to the
/// given working directory.
///
/// The cwd is forward-encoded (see [`encode_project_dir`]) and matched
/// **case-insensitively** against the directory names, because the stored
/// drive-letter case is non-deterministic (ADR-0001). All matches are returned
/// (the union), sorted for stable output. An unreadable `projects_root` yields
/// an empty result rather than an error — a missing Store is "no matches".
pub fn find_project_dirs(projects_root: &Path, cwd: &str) -> Vec<PathBuf> {
    let target = encode_project_dir(cwd).to_lowercase();
    let Ok(entries) = std::fs::read_dir(projects_root) else {
        return Vec::new();
    };
    let mut matches: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| e.file_name().to_string_lossy().to_lowercase() == target)
        .map(|e| e.path())
        .collect();
    matches.sort();
    matches
}

/// Which kind of conversation content a [`Segment`] came from. Determines how
/// a Match is labelled in output and which content-selection flags include it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Title,
}

/// A single searchable unit of text extracted from one Record, tagged with the
/// kind of content it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub role: Role,
    pub text: String,
}

/// Extract the searchable [`Segment`]s a single JSONL line contributes, for the
/// default content set (Prompts, Replies, Titles).
///
/// Parsing is **lenient**: a line that is not valid JSON, or whose shape we do
/// not recognise, contributes no Segments instead of failing. This keeps a
/// single malformed or future-versioned Record from aborting a search.
pub fn parse_line(line: &str) -> Vec<Segment> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Vec::new();
    };
    match value.get("type").and_then(|t| t.as_str()) {
        Some("user") => match value.pointer("/message/content").and_then(|c| c.as_str()) {
            Some(text) => vec![Segment { role: Role::User, text: text.to_string() }],
            None => Vec::new(),
        },
        Some("assistant") => value
            .pointer("/message/content")
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .map(|text| Segment { role: Role::Assistant, text: text.to_string() })
                    .collect()
            })
            .unwrap_or_default(),
        Some("ai-title") => match value.get("aiTitle").and_then(|t| t.as_str()) {
            Some(text) => vec![Segment { role: Role::Title, text: text.to_string() }],
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Resolve the Claude config directory (the `~/.claude` equivalent that
/// contains the `projects/` Store) from the available sources, in precedence
/// order: an explicit `--claude-dir` override, then `$CLAUDE_CONFIG_DIR`, then
/// `<home>/.claude`. Returns `None` only when none of the three is available.
///
/// Sources are passed in rather than read from the environment so the
/// precedence is pure and testable; the binary supplies the real values.
pub fn resolve_claude_dir(
    cli_override: Option<&Path>,
    env_config_dir: Option<&str>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(dir) = cli_override {
        return Some(dir.to_path_buf());
    }
    if let Some(dir) = env_config_dir {
        return Some(PathBuf::from(dir));
    }
    home.map(|h| h.join(".claude"))
}

/// The Store (Projects root) under a resolved Claude config directory.
pub fn projects_root(claude_dir: &Path) -> PathBuf {
    claude_dir.join("projects")
}

/// All Matches found within a single Session, grouped with the metadata needed
/// to display and reopen it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMatches {
    /// The Project directory name the Session lives in.
    pub project: String,
    /// The Session id (the `.jsonl` file stem).
    pub session_id: String,
    /// Full path to the Session file.
    pub path: PathBuf,
    /// The Session's AI-generated Title, if it has one.
    pub title: Option<String>,
    /// The matching Segments, in the order they appear in the Session.
    pub matches: Vec<Segment>,
}

/// Search the given Project directories for `query`, returning one
/// [`SessionMatches`] per Session that contains at least one Match.
///
/// Matching is a case-insensitive literal substring over the default content
/// set (see [`parse_line`]). Sessions with no Matches are omitted. Unreadable
/// directories and files are skipped rather than failing the whole search.
pub fn search_project_dirs(project_dirs: &[PathBuf], query: &str) -> Vec<SessionMatches> {
    let needle = query.to_lowercase();
    let mut results = Vec::new();
    for dir in project_dirs {
        let project = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut title = None;
            let mut matches = Vec::new();
            for line in content.lines() {
                for seg in parse_line(line) {
                    if seg.role == Role::Title {
                        title = Some(seg.text.clone());
                    }
                    if seg.text.to_lowercase().contains(&needle) {
                        matches.push(seg);
                    }
                }
            }
            if !matches.is_empty() {
                let session_id =
                    path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                results.push(SessionMatches {
                    project: project.clone(),
                    session_id,
                    path,
                    title,
                    matches,
                });
            }
        }
    }
    results
}

/// Maximum number of characters shown for a single Match snippet.
const SNIPPET_MAX_CHARS: usize = 200;

fn role_label(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Title => "title",
    }
}

/// Collapse a Record's text to a single readable line, truncated to
/// [`SNIPPET_MAX_CHARS`]. Internal runs of whitespace (including newlines)
/// become single spaces so a multi-line Prompt stays on one output line.
fn one_line_snippet(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > SNIPPET_MAX_CHARS {
        let head: String = collapsed.chars().take(SNIPPET_MAX_CHARS).collect();
        format!("{head}…")
    } else {
        collapsed
    }
}

/// Render search results as human- and Claude-readable text: each Session as a
/// `project · title` header followed by one `role: snippet` line per Match.
/// An empty result set renders a clear "no matches" line.
pub fn format_results(results: &[SessionMatches]) -> String {
    if results.is_empty() {
        return "No matches.\n".to_string();
    }
    let mut out = String::new();
    for s in results {
        let title = s.title.as_deref().unwrap_or("(untitled)");
        out.push_str(&format!("{} · {}\n", s.project, title));
        for m in &s.matches {
            out.push_str(&format!("  {}: {}\n", role_label(m.role), one_line_snippet(&m.text)));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn encodes_a_windows_path_the_way_claude_code_stores_it() {
        assert_eq!(
            encode_project_dir(r"E:\projects\rust\claude-code-conversation-search"),
            "E--projects-rust-claude-code-conversation-search"
        );
    }

    #[test]
    fn dots_and_consecutive_separators_each_become_their_own_dash() {
        // Real observed case: `c:\Users\jenki\.config\powershell`. The `:`, the
        // backslashes, AND the leading dot of `.config` all collapse to `-`,
        // producing the doubled dash before `config`.
        assert_eq!(
            encode_project_dir(r"c:\Users\jenki\.config\powershell"),
            "c--Users-jenki--config-powershell"
        );
    }

    #[test]
    fn alphanumerics_keep_their_case_and_existing_hyphens_pass_through() {
        assert_eq!(
            encode_project_dir(r"E:\projects\ai\agent-quiz-generator"),
            "E--projects-ai-agent-quiz-generator"
        );
    }

    #[test]
    fn finds_the_project_dir_for_the_current_cwd_case_insensitively() {
        let tmp = tempfile::tempdir().unwrap();
        let projects = tmp.path();
        // The stored directory uses a lowercase drive letter, but the cwd we
        // search from reports an uppercase one (the ADR-0001 drive wobble).
        fs::create_dir(projects.join("e--Vault2026")).unwrap();
        fs::create_dir(projects.join("C--hacking-jplag")).unwrap();

        let found = find_project_dirs(projects, r"E:\Vault2026");

        assert_eq!(found, vec![projects.join("e--Vault2026")]);
    }

    #[test]
    fn returns_empty_when_the_projects_root_does_not_exist() {
        let missing = Path::new("this-store-does-not-exist-anywhere");
        assert!(find_project_dirs(missing, r"E:\whatever").is_empty());
    }

    fn write_session(dir: &Path, id: &str, lines: &[&str]) -> PathBuf {
        let path = dir.join(format!("{id}.jsonl"));
        fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn finds_matching_segments_in_a_session_grouped_with_its_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        let path = write_session(
            &proj,
            "11111111-1111-1111-1111-111111111111",
            &[
                r#"{"type":"ai-title","aiTitle":"Borrow checker chat"}"#,
                r#"{"type":"user","message":{"role":"user","content":"how do I satisfy the BORROW checker"}}"#,
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"use a reference"}]}}"#,
            ],
        );

        let results = search_project_dirs(&[proj.clone()], "borrow");

        assert_eq!(results.len(), 1);
        let s = &results[0];
        assert_eq!(s.project, "E--projects-demo");
        assert_eq!(s.session_id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(s.path, path);
        assert_eq!(s.title.as_deref(), Some("Borrow checker chat"));
        assert_eq!(
            s.matches,
            vec![
                Segment { role: Role::Title, text: "Borrow checker chat".into() },
                Segment { role: Role::User, text: "how do I satisfy the BORROW checker".into() },
            ]
        );
    }

    fn session(project: &str, title: Option<&str>, matches: Vec<Segment>) -> SessionMatches {
        SessionMatches {
            project: project.into(),
            session_id: "11111111-2222-3333-4444-555555555555".into(),
            path: PathBuf::from("/x/11111111-2222-3333-4444-555555555555.jsonl"),
            title: title.map(Into::into),
            matches,
        }
    }

    #[test]
    fn formats_a_session_with_a_header_and_a_readable_line_per_match() {
        let results = vec![session(
            "E--projects-demo",
            Some("Borrow checker chat"),
            vec![
                Segment { role: Role::Title, text: "Borrow checker chat".into() },
                Segment { role: Role::User, text: "how do I satisfy the BORROW checker".into() },
            ],
        )];

        let out = format_results(&results);

        assert!(out.contains("E--projects-demo"), "header shows project: {out}");
        assert!(out.contains("Borrow checker chat"), "header shows title: {out}");
        assert!(
            out.contains("how do I satisfy the BORROW checker"),
            "match text shown: {out}"
        );
        assert!(out.to_lowercase().contains("user"), "match labelled by role: {out}");
    }

    #[test]
    fn collapses_multiline_match_text_onto_one_line() {
        let results = vec![session(
            "p",
            Some("t"),
            vec![Segment { role: Role::User, text: "line one\n\n   line two".into() }],
        )];

        let out = format_results(&results);

        assert!(out.contains("line one line two"), "collapsed: {out}");
        assert!(!out.contains("line one\n"), "no embedded newline in snippet: {out}");
    }

    #[test]
    fn renders_a_clear_message_when_there_are_no_matches() {
        let out = format_results(&[]);
        assert!(out.to_lowercase().contains("no match"), "{out}");
    }

    #[test]
    fn cli_override_wins_over_env_and_home() {
        let dir = resolve_claude_dir(
            Some(Path::new("/explicit/claude")),
            Some("/env/claude"),
            Some(Path::new("/home/jenki")),
        );
        assert_eq!(dir, Some(PathBuf::from("/explicit/claude")));
    }

    #[test]
    fn env_config_dir_wins_over_home_when_no_override() {
        let dir = resolve_claude_dir(None, Some("/env/claude"), Some(Path::new("/home/jenki")));
        assert_eq!(dir, Some(PathBuf::from("/env/claude")));
    }

    #[test]
    fn falls_back_to_home_dot_claude() {
        let dir = resolve_claude_dir(None, None, Some(Path::new("/home/jenki")));
        assert_eq!(dir, Some(PathBuf::from("/home/jenki/.claude")));
    }

    #[test]
    fn resolves_to_none_when_no_source_is_available() {
        assert_eq!(resolve_claude_dir(None, None, None), None);
    }

    #[test]
    fn projects_root_is_the_projects_subdir_of_the_claude_dir() {
        assert_eq!(
            projects_root(Path::new("/home/jenki/.claude")),
            PathBuf::from("/home/jenki/.claude/projects")
        );
    }

    #[test]
    fn omits_sessions_with_no_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "hit",
            &[r#"{"type":"user","message":{"role":"user","content":"set up tokio runtime"}}"#],
        );
        write_session(
            &proj,
            "miss",
            &[r#"{"type":"user","message":{"role":"user","content":"unrelated chatter"}}"#],
        );

        let results = search_project_dirs(&[proj], "tokio");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "hit");
    }

    #[test]
    fn a_query_only_present_in_thinking_does_not_match_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "only-thinking",
            &[r#"{"type":"assistant","message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"the secret password is hunter2"}
            ]}}"#],
        );

        assert!(search_project_dirs(&[proj], "hunter2").is_empty());
    }

    #[test]
    fn parses_a_user_message_into_a_user_segment() {
        let line = r#"{"type":"user","message":{"role":"user","content":"how do I borrow check"}}"#;
        assert_eq!(
            parse_line(line),
            vec![Segment { role: Role::User, text: "how do I borrow check".into() }]
        );
    }

    #[test]
    fn parses_an_ai_title_into_a_title_segment() {
        let line = r#"{"type":"ai-title","aiTitle":"Designing the search CLI","sessionId":"x"}"#;
        assert_eq!(
            parse_line(line),
            vec![Segment { role: Role::Title, text: "Designing the search CLI".into() }]
        );
    }

    #[test]
    fn skips_unparseable_and_unrecognised_lines_without_panicking() {
        assert_eq!(parse_line("this is not json at all"), vec![]);
        assert_eq!(parse_line(""), vec![]);
        assert_eq!(parse_line(r#"{"type":"queue-operation","operation":"enqueue"}"#), vec![]);
    }

    #[test]
    fn excludes_user_tool_results_from_the_default_content_set() {
        // A user Record whose content is an array is a tool_result, not a typed
        // Prompt — it must not be searched by default.
        let line = r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","content":"pasted file contents here"}
        ]}}"#;
        assert_eq!(parse_line(line), vec![]);
    }

    #[test]
    fn parses_assistant_text_blocks_and_ignores_thinking_and_tool_use() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"internal reasoning we exclude by default"},
            {"type":"text","text":"You can use a reference."},
            {"type":"tool_use","name":"Bash","input":{}},
            {"type":"text","text":"Then run it."}
        ]}}"#;
        assert_eq!(
            parse_line(line),
            vec![
                Segment { role: Role::Assistant, text: "You can use a reference.".into() },
                Segment { role: Role::Assistant, text: "Then run it.".into() },
            ]
        );
    }

    proptest::proptest! {
        /// Every output character is an ASCII alphanumeric or `-`, and the
        /// encoding is a 1:1 character map (length preserved). A future change
        /// that collapsed runs of separators would break the length invariant.
        #[test]
        fn output_is_constrained_and_length_preserving(input in ".*") {
            let encoded = encode_project_dir(&input);
            proptest::prop_assert_eq!(encoded.chars().count(), input.chars().count());
            proptest::prop_assert!(
                encoded.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            );
        }
    }
}
