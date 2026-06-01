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
