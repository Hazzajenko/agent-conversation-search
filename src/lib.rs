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
