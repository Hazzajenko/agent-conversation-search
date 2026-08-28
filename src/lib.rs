//! `agsearch` — search your local Claude Code conversation history.

mod harness;
mod session;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use regex::RegexBuilder;

use session::{AssistantBlock, Record, RecordKind, UserBlock};

pub use harness::{Harness, SessionHandle, Stores};

/// A compiled Query matcher. Both literal and regex Queries compile to one
/// [`regex::Regex`], so the search pipeline has a single match path. Literal
/// Queries are escaped first; case sensitivity is folded into the compiled
/// pattern rather than re-checked per Match.
pub struct Matcher {
    re: regex::Regex,
}

impl Matcher {
    /// Compile a [`Matcher`] for `query`. When `regex` is false the Query is
    /// matched literally (metacharacters escaped); when `case_sensitive` is
    /// false matching ignores ASCII/Unicode case. An invalid regex Query yields
    /// an error rather than a panic.
    pub fn new(query: &str, regex: bool, case_sensitive: bool) -> anyhow::Result<Matcher> {
        let pattern = if regex { query.to_string() } else { regex::escape(query) };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()?;
        Ok(Matcher { re })
    }

    /// Whether the Query matches anywhere within `text`.
    pub fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    /// The byte range of the first Match within `text`, if any. Used to center
    /// and highlight a Snippet on the Match.
    pub fn find(&self, text: &str) -> Option<(usize, usize)> {
        self.re.find(text).map(|m| (m.start(), m.end()))
    }
}

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
    project_dir_paths(projects_root)
        .into_iter()
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase() == target)
                .unwrap_or(false)
        })
        .collect()
}

/// Which kind of conversation content a [`Segment`] came from. Determines how
/// a Match is labelled in output and which content-selection flags include it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    Title,
    Thinking,
    Tool,
}

/// A single searchable unit of text extracted from one Record, tagged with the
/// kind of content it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub role: Role,
    pub text: String,
}

/// Which kinds of content a search includes beyond the always-on default set
/// (Prompts, Replies, Titles). [`segments_from_record`] emits every kind tagged
/// by [`Role`]; this is the policy the search pipeline applies to decide what to
/// actually match. The [`Default`] is the default set only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContentSet {
    /// Also search assistant `thinking` blocks.
    pub thinking: bool,
    /// Also search tool calls (`tool_use`) and tool results (`tool_result`).
    pub tools: bool,
}

impl ContentSet {
    /// Whether a [`Segment`] of the given [`Role`] is in scope for this set.
    fn includes(&self, role: Role) -> bool {
        match role {
            Role::User | Role::Assistant | Role::Title => true,
            Role::Thinking => self.thinking,
            Role::Tool => self.tools,
        }
    }
}

/// The searchable [`Segment`]s a typed [`Record`] contributes, each tagged with
/// its [`Role`]. Applies no content policy — the search pipeline decides which
/// Roles to match via a [`ContentSet`]. A user `text` block is not a Segment
/// (only Prompts, tool_results, Replies, thinking, tool calls, and Titles are),
/// matching `show`'s richer rendering being a separate projection.
fn segments_from_record(record: &Record) -> Vec<Segment> {
    match &record.kind {
        RecordKind::Prompt(text) => vec![Segment { role: Role::User, text: text.clone() }],
        RecordKind::UserBlocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                UserBlock::ToolResult { text, .. } => Some(Segment { role: Role::Tool, text: text.clone() }),
                UserBlock::Text(_) => None,
            })
            .collect(),
        RecordKind::Assistant(blocks) => blocks
            .iter()
            .map(|b| match b {
                AssistantBlock::Text(text) => Segment { role: Role::Assistant, text: text.clone() },
                AssistantBlock::Thinking(text) => Segment { role: Role::Thinking, text: text.clone() },
                AssistantBlock::ToolUse { name, input, .. } => {
                    // Both the tool name and its input arguments are searchable.
                    let input = if input.is_null() { String::new() } else { input.to_string() };
                    Segment { role: Role::Tool, text: format!("{name} {input}").trim().to_string() }
                }
            })
            .collect(),
        RecordKind::Title(text) => vec![Segment { role: Role::Title, text: text.clone() }],
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

/// Resolve the Codex config directory from `--codex-dir`, `$CODEX_HOME`, then
/// `<home>/.codex`, in that precedence order.
pub fn resolve_codex_dir(
    cli_override: Option<&Path>,
    env_codex_home: Option<&str>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    cli_override
        .map(Path::to_path_buf)
        .or_else(|| env_codex_home.map(PathBuf::from))
        .or_else(|| home.map(|directory| directory.join(".codex")))
}

/// The Store (Projects root) under a resolved Claude config directory.
pub fn projects_root(claude_dir: &Path) -> PathBuf {
    claude_dir.join("projects")
}

/// Which Projects a search covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Only the Project for the given working directory (the default).
    Current { cwd: String },
    /// Every Project in the Store.
    All,
    /// Projects whose directory name contains `name_substring` (case-insensitive).
    Project { name_substring: String },
}

/// List every Project directory under `projects_root`, sorted. Returns empty if
/// the root is unreadable.
fn project_dir_paths(projects_root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(projects_root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

/// Resolve a [`Scope`] to the concrete Project directories a search should
/// cover.
pub fn resolve_scope(projects_root: &Path, scope: &Scope) -> Vec<PathBuf> {
    match scope {
        Scope::Current { cwd } => find_project_dirs(projects_root, cwd),
        Scope::All => project_dir_paths(projects_root),
        Scope::Project { name_substring } => {
            let needle = name_substring.to_lowercase();
            project_dir_paths(projects_root)
                .into_iter()
                .filter(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_lowercase().contains(&needle))
                        .unwrap_or(false)
                })
                .collect()
        }
    }
}

/// A single Match: the matching [`Segment`] plus the turn it was found in. The
/// turn number is the same numbering [`parse_transcript`] / `show` use, so a
/// search hit points straight at `show --around <turn>` (ADR 0002). It is
/// `None` for a Title match — a Title is session metadata, not a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub turn: Option<usize>,
    pub segment: Segment,
}

/// All Matches found within a single Session, grouped with the metadata needed
/// to display and reopen it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMatches {
    pub harness: Harness,
    /// The Project directory name the Session lives in.
    pub project: String,
    /// The Session id (the `.jsonl` file stem).
    pub session_id: String,
    /// Full path to the Session file.
    pub path: PathBuf,
    /// The Session's AI-generated Title, if it has one.
    pub title: Option<String>,
    /// The newest record timestamp seen in the Session, as the raw ISO 8601
    /// string. `None` if no record carried one. ISO 8601 strings sort
    /// lexically in chronological order, so this doubles as the recency key.
    pub timestamp: Option<String>,
    /// The git branch the Session was recorded on (`gitBranch`), if any.
    pub branch: Option<String>,
    /// The real working directory the Session was recorded in (`cwd`), if any.
    /// Displayed in place of the mangled directory name (ADR 0005); `None`
    /// falls back to `project`.
    pub cwd: Option<String>,
    /// The Matches, in the order they appear in the Session.
    pub matches: Vec<Match>,
}

/// Search the given Project directories for `query`, returning one
/// [`SessionMatches`] per Session that contains at least one Match.
///
/// Matching is delegated to `matcher` over the default content set (see
/// [`segments_from_record`]). Sessions with no Matches are omitted. Unreadable
/// directories and files are skipped rather than failing the whole search.
pub fn search_project_dirs(
    project_dirs: &[PathBuf],
    matcher: &Matcher,
    content: &ContentSet,
) -> Vec<SessionMatches> {
    let sessions = enumerate_sessions(project_dirs);
    let mut results: Vec<SessionMatches> = sessions
        .par_iter()
        .filter_map(|(project, path)| search_one_session(project, path, matcher, content))
        .collect();

    // Newest Session first (timestamps are ISO 8601 strings, so reverse
    // lexical = newest-first; Sessions without a timestamp sort last). Ties
    // and missing timestamps fall back to path order for determinism
    // regardless of the parallel completion order.
    results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| a.path.cmp(&b.path)));
    results
}

/// Search Sessions enumerated by every configured Harness adapter.
pub fn search_stores(
    stores: &Stores,
    scope: &Scope,
    matcher: &Matcher,
    content: &ContentSet,
) -> Vec<SessionMatches> {
    let sessions = stores.sessions(scope, false);
    let mut results: Vec<SessionMatches> = sessions
        .par_iter()
        .filter_map(|handle| {
            let parsed = stores.parse(handle)?;
            search_parsed_session(&handle.info, parsed, stores.harness(handle), matcher, content)
        })
        .collect();
    results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| a.path.cmp(&b.path)));
    results
}

/// Search one Session already resolved across the configured Stores.
pub fn search_store_session(
    stores: &Stores,
    handle: &SessionHandle,
    matcher: &Matcher,
    content: &ContentSet,
) -> Vec<SessionMatches> {
    stores
        .parse(handle)
        .and_then(|parsed| {
            search_parsed_session(&handle.info, parsed, stores.harness(handle), matcher, content)
        })
        .into_iter()
        .collect()
}

fn search_parsed_session(
    info: &SessionInfo,
    parsed: session::Session,
    harness: Harness,
    matcher: &Matcher,
    content: &ContentSet,
) -> Option<SessionMatches> {
    let matches: Vec<Match> = parsed
        .records
        .iter()
        .flat_map(|record| {
            segments_from_record(record).into_iter().map(move |segment| Match { turn: record.turn, segment })
        })
        .filter(|found| content.includes(found.segment.role) && matcher.is_match(&found.segment.text))
        .collect();
    if matches.is_empty() {
        return None;
    }
    Some(SessionMatches {
        harness,
        project: info.project.clone(),
        session_id: info.session_id.clone(),
        path: info.path.clone(),
        title: info.title.clone(),
        timestamp: info.timestamp.clone(),
        branch: info.branch.clone(),
        cwd: info.cwd.clone(),
        matches,
    })
}

/// Every Session file as `(project name, path)` across the given Project dirs,
/// each distinct dir scanned once. Shared by the search and failure scans.
fn enumerate_sessions(project_dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut dirs: Vec<&PathBuf> = project_dirs.iter().collect();
    dirs.sort();
    dirs.dedup();
    dirs.iter()
        .flat_map(|dir| {
            let project = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("jsonl"))
                .map(move |path| (project.clone(), path))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Search a single Session file by path (the `--session` scope — ADR 0002),
/// returning 0 or 1 [`SessionMatches`]. The Project name is derived from the
/// file's parent directory.
pub fn search_session_file(path: &Path, matcher: &Matcher, content: &ContentSet) -> Vec<SessionMatches> {
    search_one_session(&session_project_name(path), path, matcher, content).into_iter().collect()
}

/// The Project name for a Session file: its parent directory's name.
fn session_project_name(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Scan a single Session file, returning its Matches if any. A file that cannot
/// be read yields `None` rather than failing the whole search.
fn search_one_session(
    project: &str,
    path: &Path,
    matcher: &Matcher,
    content: &ContentSet,
) -> Option<SessionMatches> {
    let text = std::fs::read_to_string(path).ok()?;
    let session = session::read(&text);

    // Each typed Record carries its turn number (None for a Title — metadata, not
    // a turn), so a hit points straight at `show --around <turn>` (ADR 0002). A
    // segment is kept when its Role is in scope and the Query matches.
    let matches: Vec<Match> = session
        .records
        .iter()
        .flat_map(|record| {
            segments_from_record(record).into_iter().map(move |segment| Match { turn: record.turn, segment })
        })
        .filter(|m| content.includes(m.segment.role) && matcher.is_match(&m.segment.text))
        .collect();

    if matches.is_empty() {
        return None;
    }
    let session_id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    Some(SessionMatches {
        harness: Harness::Claude,
        project: project.to_string(),
        session_id,
        path: path.to_path_buf(),
        title: session.meta.title,
        timestamp: session.meta.timestamp,
        branch: session.meta.branch,
        cwd: session.meta.cwd,
        matches,
    })
}

/// A Session's display metadata, gathered without a Query — the unit the
/// `sessions` verb lists. Carries exactly the fields [`session_header`] needs,
/// so a listing row is byte-identical to a search Session header (ADR 0004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub harness: Harness,
    /// The Project directory name the Session lives in.
    pub project: String,
    /// The Session id (the `.jsonl` file stem) — what `show <prefix>` resolves.
    pub session_id: String,
    /// Full path to the Session file (for `-l` / piping into `show -`).
    pub path: PathBuf,
    /// The Session's AI-generated Title, if it has one.
    pub title: Option<String>,
    /// The newest record timestamp (raw ISO 8601), the recency key — `None` if
    /// no record carried one, in which case the Session sorts last.
    pub timestamp: Option<String>,
    /// The git branch the Session was recorded on (`gitBranch`), if any.
    pub branch: Option<String>,
    /// The real working directory the Session was recorded in (`cwd`), if any —
    /// the human-readable name the `projects` verb shows for a Project.
    pub cwd: Option<String>,
}

/// List every Session across the given Project dirs, newest first — the
/// content-agnostic counterpart to [`search_project_dirs`] (ADR 0004). Every
/// `.jsonl` with a session-id is included; there is **no content filter**, so a
/// Session with no Messages still appears (unlike search). Files that cannot be
/// read are skipped. Sorted by the same recency key as search: newest
/// `timestamp` first, undateable Sessions last, ties by path for determinism.
pub fn list_sessions(project_dirs: &[PathBuf]) -> Vec<SessionInfo> {
    let sessions = enumerate_sessions(project_dirs);
    let mut results: Vec<SessionInfo> = sessions
        .par_iter()
        .filter_map(|(project, path)| session_info(project, path))
        .collect();
    results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| a.path.cmp(&b.path)));
    results
}

/// List Sessions enumerated by every configured Harness adapter.
pub fn list_store_sessions(stores: &Stores, scope: &Scope) -> Vec<SessionInfo> {
    stores.sessions(scope, false).into_iter().map(|session| session.info).collect()
}

/// Read a Session's header metadata (Title, newest timestamp, branch) without
/// matching a Query. Mirrors the metadata pass in [`search_one_session`] but
/// keeps every Session rather than only those with a Match. Returns `None` when
/// the file cannot be read or its stem is empty (no session-id to `show`).
fn session_info(project: &str, path: &Path) -> Option<SessionInfo> {
    let session_id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    if session_id.is_empty() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    // A listing needs only the header metadata — no Query, no Records walked
    // beyond the single parse `read` already does.
    let meta = session::read(&text).meta;
    Some(SessionInfo {
        harness: Harness::Claude,
        project: project.to_string(),
        session_id,
        path: path.to_path_buf(),
        title: meta.title,
        timestamp: meta.timestamp,
        branch: meta.branch,
        cwd: meta.cwd,
    })
}

/// A Project's display metadata — the unit the `projects` verb lists (ADR 0004,
/// ADR 0005). Identity is the on-disk directory; case-variant directories are
/// folded into one `ProjectInfo` so the listing is identical on every OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    /// Display name: the real `cwd` of the group's newest Session, falling back
    /// to the encoded directory name when no Session carries a `cwd`.
    pub name: String,
    /// How many Sessions the Project holds (summed across folded directories).
    pub session_count: usize,
    /// Newest Session `timestamp` in the group (raw ISO 8601), the recency key;
    /// `None` if no Session is dateable, in which case the Project sorts last.
    pub last_touched: Option<String>,
}

/// List the Projects across the given directories, newest-touched first (ADR
/// 0005). Identity is the on-disk directory; directories whose names are equal
/// case-insensitively are folded into one Project (ADR 0001's union rule without
/// a cwd), so the listing behaves the same on case-insensitive (Windows/macOS)
/// and case-sensitive (Linux/WSL) Stores. A directory with no listable Session
/// contributes nothing. Each Project's display name is the `cwd` of its newest
/// Session, falling back to the encoded directory name.
pub fn list_projects(project_dirs: &[PathBuf]) -> Vec<ProjectInfo> {
    // Reuse the Session listing: it already carries each Session's directory
    // name, newest timestamp, and cwd — everything a Project row needs. The
    // fold itself is a pure step over that data (see [`group_projects`]).
    group_projects(list_sessions(project_dirs))
}

/// List Projects derived from Sessions across every configured Store.
pub fn list_store_projects(stores: &Stores, scope: &Scope) -> Vec<ProjectInfo> {
    group_projects(list_store_sessions(stores, scope))
}

/// Fold a flat list of Sessions into Projects: group by the lower-cased
/// directory name (ADR 0001's union key without a cwd), summing counts and
/// taking the newest `timestamp` / its `cwd` per group. Split from
/// [`list_projects`] because the case-variant fold cannot be staged on a
/// case-insensitive filesystem (the two directories can't coexist), so the
/// logic is tested here on constructed data rather than the real Store.
fn group_projects(sessions: Vec<SessionInfo>) -> Vec<ProjectInfo> {
    // Group by the lower-cased directory name (the case-fold union key).
    let mut groups: HashMap<String, Vec<SessionInfo>> = HashMap::new();
    for s in sessions {
        groups.entry(s.project.to_lowercase()).or_default().push(s);
    }

    let mut projects: Vec<(String, ProjectInfo)> = groups
        .into_iter()
        .map(|(key, mut members)| {
            // Newest Session first within the group (same comparator as
            // list_sessions), so member[0] supplies the display cwd and date.
            members.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| a.path.cmp(&b.path)));
            let newest = &members[0];
            let name = newest.cwd.clone().unwrap_or_else(|| newest.project.clone());
            let info = ProjectInfo {
                name,
                session_count: members.len(),
                last_touched: newest.timestamp.clone(),
            };
            (key, info)
        })
        .collect();

    // Newest-touched first; ties broken by the lower-cased group key.
    projects.sort_by(|a, b| b.1.last_touched.cmp(&a.1.last_touched).then_with(|| a.0.cmp(&b.0)));
    projects.into_iter().map(|(_, p)| p).collect()
}

/// Maximum number of characters shown for a single Match snippet.
const SNIPPET_MAX_CHARS: usize = 200;

fn role_label(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Title => "title",
        Role::Thinking => "thinking",
        Role::Tool => "tool",
    }
}

/// Build a one-line Snippet for a Match: collapse internal whitespace (so a
/// multi-line Record stays on one line), then take a window of at most
/// [`SNIPPET_MAX_CHARS`] characters centered on the Match, adding `…` at either
/// end that was cut. If the Match cannot be located in the collapsed text
/// (e.g. it spanned whitespace that collapsed), the window falls back to the
/// head of the text.
fn centered_snippet(text: &str, matcher: &Matcher, color: bool) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = collapsed.chars().collect();

    // Locate the Match in char indices (regex gives byte offsets).
    let span = matcher.find(&collapsed).map(|(b0, b1)| {
        (collapsed[..b0].chars().count(), collapsed[..b1].chars().count())
    });

    // Window: the whole text if it fits, otherwise SNIPPET_MAX_CHARS centered
    // on the Match (or the head if the Match could not be located).
    let (start, end) = if chars.len() <= SNIPPET_MAX_CHARS {
        (0, chars.len())
    } else {
        let (ms, me) = span.unwrap_or((0, 0));
        let pad = SNIPPET_MAX_CHARS.saturating_sub(me - ms) / 2;
        let mut s = ms.saturating_sub(pad);
        let e = (s + SNIPPET_MAX_CHARS).min(chars.len());
        s = e.saturating_sub(SNIPPET_MAX_CHARS).min(s);
        (s, e)
    };

    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    push_window(&mut out, &chars, start, end, span, color);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// Append `chars[start..end]` to `out`, highlighting the portion that overlaps
/// the Match `span` (in char indices) when `color` is set.
fn push_window(
    out: &mut String,
    chars: &[char],
    start: usize,
    end: usize,
    span: Option<(usize, usize)>,
    color: bool,
) {
    use owo_colors::OwoColorize;

    let highlight = span.filter(|_| color).map(|(ms, me)| (ms.max(start), me.min(end)));
    match highlight {
        Some((hs, he)) if hs < he => {
            out.extend(&chars[start..hs]);
            let matched: String = chars[hs..he].iter().collect();
            out.push_str(&matched.black().on_bright_yellow().to_string());
            out.extend(&chars[he..end]);
        }
        _ => out.extend(&chars[start..end]),
    }
}

/// The `YYYY-MM-DD` date prefix of an ISO 8601 timestamp, or `None` if the
/// string is too short to carry one. Slicing avoids pulling in a date library.
fn date_prefix(timestamp: &str) -> Option<&str> {
    timestamp.get(..10)
}

/// The short session-id shown in headers and the `show` hint: the first 8
/// characters of the session-id (git-style), or the whole id if shorter. This
/// is what `show <prefix>` resolves against (ADR 0002).
fn short_id(session_id: &str) -> String {
    session_id.chars().take(8).collect()
}

/// The one-line Session header shared by search and `--failed`: leads with the
/// short session-id (paste-able into `show`), then `project · title · date ·
/// branch`, omitting date/branch when absent.
fn session_header(
    short: &str,
    project: &str,
    title: Option<&str>,
    timestamp: Option<&str>,
    branch: Option<&str>,
) -> String {
    let mut header = vec![short.to_string(), project.to_string(), title.unwrap_or("(untitled)").to_string()];
    if let Some(date) = timestamp.and_then(date_prefix) {
        header.push(date.to_string());
    }
    if let Some(branch) = branch {
        header.push(branch.to_string());
    }
    header.join(" · ")
}

/// Render just the matching Session file paths, one per line (grep `-l` style),
/// in result order — for piping into other tools. An empty result set renders
/// nothing.
pub fn format_paths(results: &[SessionMatches]) -> String {
    let mut out = String::new();
    for s in results {
        out.push_str(&s.path.to_string_lossy());
        out.push('\n');
    }
    out
}

/// Render a Session listing (the `sessions` verb): one
/// `short-id · project · title · date · branch` header per Session — the same
/// line search prints above its Snippets (ADR 0004) — newest first. An empty
/// listing renders a clear "no sessions" line. The header carries no colour
/// (search colours only Snippets), so this output is plain text.
pub fn format_sessions(sessions: &[SessionInfo]) -> String {
    if sessions.is_empty() {
        return "No sessions.\n".to_string();
    }
    let mut out = String::new();
    for s in sessions {
        out.push_str(s.harness.as_str());
        out.push_str(" · ");
        out.push_str(&session_header(
            &short_id(&s.session_id),
            // Prefer the real cwd over the mangled directory name — the reason
            // the tool exists — falling back as `projects` does (ADR 0005).
            s.cwd.as_deref().unwrap_or(&s.project),
            s.title.as_deref(),
            s.timestamp.as_deref(),
            s.branch.as_deref(),
        ));
        out.push('\n');
    }
    out
}

/// Render a Project listing (the `projects` verb, ADR 0004): one
/// `name · N sessions · date` row per Project, newest-touched first. The date is
/// omitted when the Project has no dateable Session. An empty listing renders a
/// clear "no projects" line. Plain text (no colour), like `sessions`.
pub fn format_projects(projects: &[ProjectInfo]) -> String {
    if projects.is_empty() {
        return "No projects.\n".to_string();
    }
    let mut out = String::new();
    for p in projects {
        let noun = if p.session_count == 1 { "session" } else { "sessions" };
        out.push_str(&format!("{} · {} {noun}", p.name, p.session_count));
        if let Some(date) = p.last_touched.as_deref().and_then(date_prefix) {
            out.push_str(" · ");
            out.push_str(date);
        }
        out.push('\n');
    }
    out
}

/// Render just the listed Session file paths, one per line, in listing order —
/// the `sessions -l` counterpart to [`format_paths`], for piping into `show -`.
pub fn format_session_paths(sessions: &[SessionInfo]) -> String {
    let mut out = String::new();
    for s in sessions {
        out.push_str(&s.path.to_string_lossy());
        out.push('\n');
    }
    out
}

/// Render search results as human- and Claude-readable text: each Session as a
/// `short-id · project · title · date · branch` header followed by one
/// `[turn] role: snippet` line per Match. At most `max_per_session` Matches are
/// shown per Session (`0` = unlimited), with an actionable `… +N more  ›
/// agsearch show <id>` line when some are hidden. An empty result set renders a
/// clear "no matches" line.
pub fn format_results(
    results: &[SessionMatches],
    matcher: &Matcher,
    max_per_session: usize,
    color: bool,
) -> String {
    if results.is_empty() {
        return "No matches.\n".to_string();
    }
    let mut out = String::new();
    for s in results {
        let short = short_id(&s.session_id);
        out.push_str(s.harness.as_str());
        out.push_str(" · ");
        out.push_str(&session_header(
            &short,
            // Prefer the real cwd over the mangled directory name (ADR 0005),
            // as `sessions` and `projects` do.
            s.cwd.as_deref().unwrap_or(&s.project),
            s.title.as_deref(),
            s.timestamp.as_deref(),
            s.branch.as_deref(),
        ));
        out.push('\n');

        // A cap of 0 means show every Match.
        let shown = if max_per_session == 0 {
            s.matches.len()
        } else {
            s.matches.len().min(max_per_session)
        };
        for m in &s.matches[..shown] {
            // Lead each Match with its turn number so it points at
            // `show --around <turn>`; a Title match has no turn.
            let turn = match m.turn {
                Some(turn) => format!("[{turn}] "),
                None => String::new(),
            };
            out.push_str(&format!(
                "  {turn}{}: {}\n",
                role_label(m.segment.role),
                centered_snippet(&m.segment.text, matcher, color)
            ));
        }
        let hidden = s.matches.len() - shown;
        if hidden > 0 {
            // Turn the overflow into an actionable hint at the rest.
            out.push_str(&format!("  … +{hidden} more  ›  agsearch show {short}\n"));
        }
        out.push('\n');
    }
    out
}

// --- show: resolving a Session and rendering it as a Transcript ---------

/// The outcome of resolving a git-style session-id prefix against the whole
/// Store (see ADR 0002). A session-id is globally unique, so resolution scans
/// every Project — you often reopen a Session from a different Project than the
/// cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRef {
    /// Exactly one Session matched the prefix; here is its file path.
    Unique(PathBuf),
    /// More than one Session shares the prefix; here are their full session-ids
    /// (sorted) so the caller can ask the user to disambiguate.
    Ambiguous(Vec<String>),
    /// No Session in the Store starts with the prefix.
    NotFound,
}

/// Result of resolving a Session id across every configured Store.
pub enum StoreSessionRef {
    Unique(SessionHandle),
    Ambiguous(Vec<String>),
    NotFound,
}

pub fn resolve_store_session_prefix(stores: &Stores, prefix: &str) -> StoreSessionRef {
    let sessions = stores.all_sessions(false);
    if let Some(exact) = sessions.iter().find(|session| session.info.session_id == prefix) {
        return StoreSessionRef::Unique(exact.clone());
    }
    let mut matches: Vec<SessionHandle> = sessions
        .into_iter()
        .filter(|session| session.info.session_id.starts_with(prefix))
        .collect();
    matches.sort_by(|a, b| a.info.session_id.cmp(&b.info.session_id));
    match matches.len() {
        0 => StoreSessionRef::NotFound,
        1 => StoreSessionRef::Unique(matches.remove(0)),
        _ => StoreSessionRef::Ambiguous(matches.into_iter().map(|session| session.info.session_id).collect()),
    }
}

/// Resolve `prefix` to a single Session file across the whole Store, git-style.
///
/// A full session-id that exactly equals an existing stem wins outright (so a
/// complete id is never reported ambiguous against a longer one). Otherwise the
/// prefix must match exactly one Session stem. Unreadable directories are
/// skipped rather than failing resolution.
pub fn resolve_session_prefix(projects_root: &Path, prefix: &str) -> SessionRef {
    let mut matches: Vec<(String, PathBuf)> = Vec::new();
    for dir in project_dir_paths(projects_root) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            if stem == prefix {
                return SessionRef::Unique(path); // exact id wins, git-style
            }
            if stem.starts_with(prefix) {
                matches.push((stem, path));
            }
        }
    }
    match matches.len() {
        0 => SessionRef::NotFound,
        1 => SessionRef::Unique(matches.pop().unwrap().1),
        _ => {
            matches.sort();
            SessionRef::Ambiguous(matches.into_iter().map(|(stem, _)| stem).collect())
        }
    }
}

/// Which kind of turn a [`Turn`] is, and therefore how it is labelled. A user
/// Message whose content is a plain string is a [`Prompt`](TurnKind::Prompt);
/// a user Message carrying only `tool_result` blocks is mechanically-generated
/// [`ToolOutput`](TurnKind::ToolOutput) and renders without a "you" header,
/// because the person did not type it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnKind {
    /// A user Message the person typed (rendered under `you`).
    Prompt,
    /// An assistant Message (rendered under `claude`).
    Reply,
    /// A user Message that is purely tool results (rendered header-less).
    ToolOutput,
}

/// One renderable block within a [`Turn`]. Carries enough structure for the
/// Transcript renderer to show compact tool one-liners and flag Failures —
/// unlike a [`Segment`], which flattens everything to searchable text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnBlock {
    /// A Prompt or Reply text block.
    Text(String),
    /// An assistant `thinking` block (collapsed unless `--thinking`).
    Thinking(String),
    /// A tool call: the tool name plus its key argument, if any.
    ToolUse { name: String, arg: Option<String> },
    /// A tool result, with whether it errored and the tool it came from (joined
    /// via `tool_use_id`), if known.
    ToolResult { is_error: bool, tool: Option<String>, text: String },
}

/// A single turn of a Transcript — one user or assistant Message Record, with
/// its 1-based turn number (in Message order, the same numbering search emits
/// for the handoff) and its renderable blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub number: usize,
    pub kind: TurnKind,
    pub blocks: Vec<TurnBlock>,
}

/// Parse a whole Session file into its Transcript turns (Messages only — the
/// noise Records are dropped), projecting the typed Records from [`session::read`]
/// (ADR 0006). A failed `tool_result` is labelled with the tool that produced it,
/// joined through [`session::tool_index`].
pub fn parse_transcript(session_text: &str) -> Vec<Turn> {
    let session = session::read(session_text);
    turns_from_session(&session)
}

/// Parse a resolved Session through its Harness adapter and project it as a Transcript.
pub fn parse_store_transcript(stores: &Stores, handle: &SessionHandle) -> Option<Vec<Turn>> {
    stores.parse(handle).map(|session| turns_from_session(&session))
}

fn turns_from_session(session: &session::Session) -> Vec<Turn> {
    let tools = session::tool_index(&session.records);
    session.records.iter().filter_map(|record| turn_from_record(record, &tools)).collect()
}

/// Project one typed [`Record`] into a renderable [`Turn`], or `None` when it
/// contributes none — a Title, or a Message whose blocks all render nothing (an
/// empty Prompt, a signature-only thinking block). The Record already carries its
/// turn number, so the find→read handoff coordinate (ADR 0002) is never
/// recomputed here.
fn turn_from_record(record: &Record, tools: &HashMap<String, (String, Option<String>)>) -> Option<Turn> {
    let (kind, blocks) = match &record.kind {
        RecordKind::Title(_) => return None,
        RecordKind::Prompt(text) => {
            if text.trim().is_empty() {
                return None;
            }
            (TurnKind::Prompt, vec![TurnBlock::Text(text.clone())])
        }
        RecordKind::UserBlocks(blocks) => {
            let blocks: Vec<TurnBlock> =
                blocks.iter().filter_map(|b| user_turn_block(b, tools)).collect();
            if blocks.is_empty() {
                return None;
            }
            // A user Message that is *only* tool results is mechanical output,
            // not something the person typed.
            let kind = if blocks.iter().all(|b| matches!(b, TurnBlock::ToolResult { .. })) {
                TurnKind::ToolOutput
            } else {
                TurnKind::Prompt
            };
            (kind, blocks)
        }
        RecordKind::Assistant(blocks) => {
            let blocks: Vec<TurnBlock> = blocks.iter().filter_map(assistant_turn_block).collect();
            if blocks.is_empty() {
                return None;
            }
            (TurnKind::Reply, blocks)
        }
    };
    Some(Turn { number: record.turn.unwrap_or(0), kind, blocks })
}

/// Project one [`UserBlock`] into a renderable [`TurnBlock`], dropping blank
/// text. A `tool_result` is labelled with the tool that produced it, joined
/// through `tools`.
fn user_turn_block(block: &UserBlock, tools: &HashMap<String, (String, Option<String>)>) -> Option<TurnBlock> {
    match block {
        UserBlock::Text(text) if !text.trim().is_empty() => Some(TurnBlock::Text(text.clone())),
        UserBlock::Text(_) => None,
        UserBlock::ToolResult { is_error, tool_use_id, text } => {
            let tool = tool_use_id.as_deref().and_then(|id| tools.get(id)).map(|(name, _)| name.clone());
            Some(TurnBlock::ToolResult { is_error: *is_error, tool, text: text.clone() })
        }
    }
}

/// Project one [`AssistantBlock`] into a renderable [`TurnBlock`], or `None` for
/// blocks that render nothing (blank text, or a signature-only thinking block).
fn assistant_turn_block(block: &AssistantBlock) -> Option<TurnBlock> {
    match block {
        AssistantBlock::Text(text) if !text.trim().is_empty() => Some(TurnBlock::Text(text.clone())),
        AssistantBlock::Text(_) => None,
        AssistantBlock::Thinking(text) if !text.trim().is_empty() => Some(TurnBlock::Thinking(text.clone())),
        AssistantBlock::Thinking(_) => None,
        AssistantBlock::ToolUse { name, input, .. } => {
            let name = if name.is_empty() { "tool".to_string() } else { name.clone() };
            Some(TurnBlock::ToolUse { name, arg: session::tool_key_arg(input) })
        }
    }
}

/// Width that Transcript prose is word-wrapped to before its 2-space indent.
const WRAP_WIDTH: usize = 88;
/// Maximum characters of a tool one-liner (call argument or result) shown.
const TOOL_LINE_MAX: usize = 160;

/// Collapse `text`'s internal whitespace to single spaces and truncate to at
/// most `max` characters, appending `…` when cut. Used for the one-liner tool
/// call/result renderings.
fn one_line(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = collapsed.chars().collect();
    if chars.len() <= max {
        collapsed
    } else {
        let cut: String = chars[..max].iter().collect();
        format!("{cut}…")
    }
}

/// Greedy word-wrap `text` to `width` columns, preserving its existing line
/// breaks (each input line is wrapped independently; blank lines are kept).
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + 1 + word.chars().count() <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        lines.push(current);
    }
    lines
}

/// Append `text` word-wrapped and indented 2 spaces, one line per wrapped line.
fn push_wrapped(out: &mut String, text: &str) {
    for line in wrap(text, WRAP_WIDTH) {
        if line.is_empty() {
            out.push('\n');
        } else {
            out.push_str("  ");
            out.push_str(&line);
            out.push('\n');
        }
    }
}

/// Render parsed [`Turn`]s as a human-readable Transcript (see CONTEXT.md):
/// `you` / `claude` speaker headers, prose wrapped readably, tool calls as
/// compact one-liners, Failures flagged loudly with `✗ … FAILED`. Thinking is
/// collapsed to a one-line count unless `show_thinking` is set. A Session with
/// no Messages renders a clear placeholder.
pub fn format_transcript(turns: &[Turn], show_thinking: bool) -> String {
    if turns.is_empty() {
        return "(no messages)\n".to_string();
    }
    let mut out = String::new();
    for turn in turns {
        render_turn(&mut out, turn, show_thinking);
    }
    out
}

/// Render one [`Turn`]: its speaker header (tool output has none) followed by
/// its blocks, then a trailing blank line.
fn render_turn(out: &mut String, turn: &Turn, show_thinking: bool) {
    match turn.kind {
        TurnKind::Prompt => out.push_str("you\n"),
        TurnKind::Reply => out.push_str("claude\n"),
        TurnKind::ToolOutput => {} // mechanical tool output: no speaker header
    }
    for block in &turn.blocks {
        render_block(out, block, show_thinking);
    }
    out.push('\n');
}

/// The contiguous slice of `turns` to render for a `--around T --context N`
/// window, plus how many turns fall before and after it. Turns are
/// number-sorted, so the window is a slice. The range is `[T-N, T+N]` on the
/// turn-number axis (the numbering search emits), clamping naturally at the
/// ends of the Session; a target past the last turn yields an empty window.
pub fn window_turns(turns: &[Turn], around: usize, context: usize) -> (&[Turn], usize, usize) {
    let lo = around.saturating_sub(context);
    let hi = around.saturating_add(context);
    let start = turns.iter().position(|t| t.number >= lo).unwrap_or(turns.len());
    let end = turns.iter().rposition(|t| t.number <= hi).map_or(start, |i| i + 1);
    (&turns[start..end], start, turns.len() - end)
}

/// Render a windowed Transcript: the turns around `around` (see
/// [`window_turns`]), bracketed by indicators of how many turns are hidden
/// above and below so the reader knows where they are in the Session.
pub fn format_windowed(turns: &[Turn], around: usize, context: usize, show_thinking: bool) -> String {
    let (window, above, below) = window_turns(turns, around, context);
    let mut out = String::new();
    if above > 0 {
        let unit = if above == 1 { "turn" } else { "turns" };
        out.push_str(&format!(
            "… {above} earlier {unit} hidden — omit --around for the whole transcript\n\n"
        ));
    }
    if window.is_empty() {
        out.push_str("(no turns in this window)\n");
    } else {
        for turn in window {
            render_turn(&mut out, turn, show_thinking);
        }
    }
    if below > 0 {
        let unit = if below == 1 { "turn" } else { "turns" };
        out.push_str(&format!("… {below} later {unit} hidden\n"));
    }
    out
}

/// Render one [`TurnBlock`] into the Transcript.
fn render_block(out: &mut String, block: &TurnBlock, show_thinking: bool) {
    match block {
        TurnBlock::Text(text) => push_wrapped(out, text),
        TurnBlock::Thinking(text) => {
            if show_thinking {
                out.push_str("  [thinking]\n");
                push_wrapped(out, text);
            } else {
                let lines = text.lines().count().max(1);
                let unit = if lines == 1 { "line" } else { "lines" };
                out.push_str(&format!("  [thinking: {lines} {unit} hidden — pass --thinking]\n"));
            }
        }
        TurnBlock::ToolUse { name, arg } => match arg {
            Some(arg) => out.push_str(&format!("  → {name} {}\n", one_line(arg, TOOL_LINE_MAX))),
            None => out.push_str(&format!("  → {name}\n")),
        },
        TurnBlock::ToolResult { is_error, tool, text } => {
            if *is_error {
                let label = tool.as_deref().unwrap_or("tool");
                out.push_str(&format!("  ✗ {label} FAILED: {}\n", one_line(text, TOOL_LINE_MAX)));
            } else {
                out.push_str(&format!("  ← {}\n", one_line(text, TOOL_LINE_MAX)));
            }
        }
    }
}

// --- failure analysis: finding failed tool calls by structure -----------

/// One classifier entry: a `needle` substring that identifies a failure class,
/// and the short `label` shown as its signature in `stats`. For most entries the
/// label is the needle itself; harness-noise entries use a terse label distinct
/// from the verbose message they match (e.g. `rejected` for "tool use was
/// rejected"). See [`FAILURE_MARKERS`].
struct FailureMarker {
    needle: &'static str,
    label: &'static str,
    /// Whether `label` is a stable, **universal** signature — Claude Code's own
    /// tool/harness vocabulary and OS-level errors, identical for every user —
    /// or merely a display *hint* for [`salient_line`] (which line to highlight)
    /// whose Failures are grouped by [`structural_signature`] instead. Keeping a
    /// language's error syntax (`error[`, `Traceback`, …) out of the universal
    /// set is what stops the `stats` table being overfit to one stack.
    universal: bool,
}

/// A universal marker: its `label` is a stable `stats` signature for everyone.
const fn fm(needle: &'static str, label: &'static str) -> FailureMarker {
    FailureMarker { needle, label, universal: true }
}

/// A display-only hint: used by [`salient_line`] to pick the informative line,
/// but its Failures group by structural shape, not by this `label`.
const fn hint(needle: &'static str, label: &'static str) -> FailureMarker {
    FailureMarker { needle, label, universal: false }
}

/// Markers that classify a tool Failure, in **priority order**. Applied
/// top-to-bottom: the first whose `needle` matches any line wins, so a compiler
/// `error[` outranks a generic `Error:` regardless of where each appears. A
/// fixed, documented lookup table — not a relevance ranker (ADR 0002 / 0003).
///
/// Two kinds of entry (see [`FailureMarker::universal`]):
/// - [`fm`] — **universal** labels: Claude Code's own tool/harness vocabulary
///   and OS-level shell errors. These are identical for every user, so they make
///   stable `stats` buckets. Where the identifying phrase sits amid variable text
///   (a filename, a command name) a fixed label also beats structural grouping.
/// - [`hint`] — **display hints**: programming-language error syntax. Kept so
///   [`salient_line`] highlights the right line, but their Failures group by
///   [`structural_signature`], so the table is never overfit to one stack.
///
/// Ordering: specific code errors first (display priority), then universal tool
/// errors (issue 14), then harness noise last. (Real data: the naive "first
/// line" is just `Exit code 101`; the real error is several lines down — issue 11.)
const FAILURE_MARKERS: &[FailureMarker] = &[
    // Programming-language error syntax: display hints only, grouped structurally.
    hint("error[", "error["),
    hint("panicked at", "panicked at"),
    hint("assertion failed", "assertion failed"),
    hint("assertion `", "assertion `"),
    hint("Error:", "Error:"),
    fm("does not exist", "does not exist"),
    fm("No such file", "No such file"),
    fm("unexpected EOF", "unexpected EOF"),
    fm("command not found", "command not found"),
    hint("fatal:", "fatal:"),
    hint("Traceback", "Traceback"),
    hint("error:", "error:"),
    // Common tool errors that match no code marker (issue 14) — universal.
    fm("String to replace not found", "String to replace not found"),
    fm("has not been read", "has not been read"),
    fm("exceeds maximum allowed tokens", "exceeds maximum allowed tokens"),
    fm("is not recognized", "is not recognized"),
    fm("Blocked:", "Blocked"),
    // Harness noise: structurally is_error, but the tool did not error. Bucketed
    // (not filtered) under a terse label so they are visible but not conflated.
    fm("tool use was rejected", "rejected"),
    fm("temporarily unavailable", "unavailable"),
    fm("Cancelled: parallel tool call", "cancelled"),
];

/// Strip ANSI escape sequences from `text` so a Failure's error line is plain.
fn strip_ansi(text: &str) -> String {
    anstream::adapter::strip_str(text).to_string()
}

/// Normalise a Failure's error text for classification and display: unwrap the
/// `<tool_use_error>…</tool_use_error>` envelope the tools wrap their messages in
/// (otherwise the inner message never matches a marker — issue 14), then strip
/// ANSI codes. Shared by [`failure_signature`] and [`salient_line`] so the
/// aggregate table and the per-Failure list always agree.
fn clean_error_text(error_text: &str) -> String {
    let unwrapped = error_text.replace("<tool_use_error>", "").replace("</tool_use_error>", "");
    strip_ansi(&unwrapped)
}

/// The highest-priority [`FAILURE_MARKERS`] entry the (cleaned) error text
/// matches, or `None` when none do.
fn matched_marker(error_text: &str) -> Option<&'static FailureMarker> {
    let cleaned = clean_error_text(error_text);
    FAILURE_MARKERS.iter().find(|m| cleaned.lines().any(|l| l.contains(m.needle)))
}

/// The Failure's **signature** — the grouping key for `stats` (ADR 0003 / 0007):
/// the matched marker's `label` when that marker is [universal](FailureMarker::universal),
/// otherwise the [`structural_signature`] of the same salient line a language
/// hint (or no marker) would highlight. So the table buckets on Claude Code's own
/// vocabulary where it can, and on error *shape* — never one stack's syntax —
/// everywhere else. `(no marker)` survives only for a Failure with no error text.
fn failure_signature(error_text: &str) -> String {
    match matched_marker(error_text) {
        Some(m) if m.universal => m.label.to_string(),
        _ => {
            let shape = structural_signature(&salient_line(error_text));
            if shape.is_empty() { "(no marker)".to_string() } else { shape }
        }
    }
}

/// Reduce an error line to its **shape**: mask the volatile parts — file paths
/// (any token with a `/` or `\`) become `<path>`, runs of digits collapse to a
/// single `N` — so two errors of the same form group together without the table
/// hard-coding any language's syntax. Truncated so one long line can't dominate.
///
/// `thread 'main' panicked at src/lib.rs:5:9:` → `thread 'main' panicked at <path>`
/// `error[E0433]: cannot find type` → `error[EN]: cannot find type`
fn structural_signature(line: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for tok in line.split_whitespace() {
        if tok.contains('/') || tok.contains('\\') {
            parts.push("<path>".to_string());
            continue;
        }
        let mut masked = String::with_capacity(tok.len());
        let mut in_digits = false;
        for c in tok.chars() {
            if c.is_ascii_digit() {
                if !in_digits {
                    masked.push('N');
                }
                in_digits = true;
            } else {
                masked.push(c);
                in_digits = false;
            }
        }
        parts.push(masked);
    }
    let joined = parts.join(" ");
    const MAX: usize = 60;
    if joined.chars().count() > MAX {
        joined.chars().take(MAX).collect::<String>() + "…"
    } else {
        joined
    }
}

/// The single most informative line of a Failure's error text: the first line
/// containing the matched marker's needle, falling back to the last non-empty
/// line when none match. The `<tool_use_error>` envelope and ANSI codes are
/// stripped first.
fn salient_line(error_text: &str) -> String {
    let cleaned = clean_error_text(error_text);
    let lines: Vec<&str> = cleaned.lines().collect();
    match matched_marker(error_text) {
        Some(m) => lines
            .iter()
            .find(|l| l.contains(m.needle))
            .map_or_else(String::new, |l| l.trim().to_string()),
        None => lines.iter().rev().map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or("").to_string(),
    }
}

/// The exit code embedded in a Failure's error text (`Exit code N`, as Bash and
/// cargo emit), or `None` when the text carries no such line.
fn exit_code(error_text: &str) -> Option<i64> {
    let marker = "Exit code ";
    let rest = &error_text[error_text.find(marker)? + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// A Failure (see CONTEXT.md): a `tool_result` that errored, joined via its
/// `tool_use_id` back to the `tool_use` that triggered it for the tool name and
/// command. Found by structure, not by a Query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The turn the `tool_result` Record sits at (same numbering as search/show).
    pub turn: Option<usize>,
    /// The tool that failed, joined from its `tool_use` (e.g. `Bash`).
    pub tool: Option<String>,
    /// The tool's key argument (command / file_path / …), joined from `tool_use`.
    pub command: Option<String>,
    /// The exit code parsed from the error text, if it carries one.
    pub exit_code: Option<i64>,
    /// The full error text (the `tool_result`'s content).
    pub error_text: String,
}

/// All Failures found within a single Session, grouped with the metadata needed
/// to display and reopen it — the failure-analysis analogue of [`SessionMatches`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFailures {
    pub project: String,
    pub session_id: String,
    pub path: PathBuf,
    pub title: Option<String>,
    pub timestamp: Option<String>,
    pub branch: Option<String>,
    /// The real working directory the Session was recorded in (`cwd`), if any —
    /// displayed in place of the mangled directory name (ADR 0005).
    pub cwd: Option<String>,
    pub failures: Vec<Failure>,
}

/// Scan the given Project dirs for Failures, returning one [`SessionFailures`]
/// per Session that has at least one. When `matcher` is `Some`, only Failures
/// whose command or error text matches the Query are kept (the Query is
/// optional under `--failed`). Newest Session first, like search.
pub fn failed_in_project_dirs(
    project_dirs: &[PathBuf],
    matcher: Option<&Matcher>,
) -> Vec<SessionFailures> {
    let sessions = enumerate_sessions(project_dirs);
    let mut results: Vec<SessionFailures> = sessions
        .par_iter()
        .filter_map(|(project, path)| failures_in_one_session(project, path, matcher))
        .collect();
    results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| a.path.cmp(&b.path)));
    results
}

/// Scan Sessions enumerated by every configured Harness adapter for Failures.
pub fn failed_in_stores(
    stores: &Stores,
    scope: &Scope,
    matcher: Option<&Matcher>,
) -> Vec<SessionFailures> {
    let sessions = stores.sessions(scope, false);
    let mut results: Vec<SessionFailures> = sessions
        .par_iter()
        .filter_map(|handle| {
            let parsed = stores.parse(handle)?;
            failures_in_parsed_session(&handle.info, parsed, matcher)
        })
        .collect();
    results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| a.path.cmp(&b.path)));
    results
}

pub fn failed_in_store_session(
    stores: &Stores,
    handle: &SessionHandle,
    matcher: Option<&Matcher>,
) -> Vec<SessionFailures> {
    stores
        .parse(handle)
        .and_then(|parsed| failures_in_parsed_session(&handle.info, parsed, matcher))
        .into_iter()
        .collect()
}

/// List Failures within a single Session file by path, so `--failed` composes
/// with the `--session` scope. Returns 0 or 1 [`SessionFailures`].
pub fn failed_in_session_file(path: &Path, matcher: Option<&Matcher>) -> Vec<SessionFailures> {
    failures_in_one_session(&session_project_name(path), path, matcher).into_iter().collect()
}

/// Scan a single Session for Failures, joining each errored `tool_result` to
/// its `tool_use`. A file that cannot be read yields `None`.
fn failures_in_one_session(
    project: &str,
    path: &Path,
    matcher: Option<&Matcher>,
) -> Option<SessionFailures> {
    let text = std::fs::read_to_string(path).ok()?;
    let session = session::read(&text);
    let info = SessionInfo {
        harness: Harness::Claude,
        project: project.to_string(),
        session_id: path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
        path: path.to_path_buf(),
        title: session.meta.title.clone(),
        timestamp: session.meta.timestamp.clone(),
        branch: session.meta.branch.clone(),
        cwd: session.meta.cwd.clone(),
    };
    failures_in_parsed_session(&info, session, matcher)
}

fn failures_in_parsed_session(
    info: &SessionInfo,
    session: session::Session,
    matcher: Option<&Matcher>,
) -> Option<SessionFailures> {
    // The join a Failure needs: tool_result id -> the tool_use that produced it.
    let tools = session::tool_index(&session.records);

    // An errored tool_result Block becomes a Failure, joined to its tool_use for
    // the tool name and command. The Record already carries its turn (the
    // tool_result sits in a user Message), so no renumbering here.
    let mut failures: Vec<Failure> = session
        .records
        .iter()
        .filter_map(|record| match &record.kind {
            RecordKind::UserBlocks(blocks) => Some((record.turn, blocks)),
            _ => None,
        })
        .flat_map(|(turn, blocks)| blocks.iter().map(move |block| (turn, block)))
        .filter_map(|(turn, block)| match block {
            UserBlock::ToolResult { is_error: true, tool_use_id, text } => {
                let (tool, command) = tool_use_id
                    .as_deref()
                    .and_then(|id| tools.get(id))
                    .map_or((None, None), |(name, command)| (Some(name.clone()), command.clone()));
                Some(Failure {
                    turn,
                    tool,
                    command,
                    exit_code: exit_code(text),
                    error_text: text.clone(),
                })
            }
            _ => None,
        })
        .collect();

    // Query filter (optional under --failed): match command or error text.
    if let Some(matcher) = matcher {
        failures.retain(|f| {
            f.command.as_deref().is_some_and(|c| matcher.is_match(c)) || matcher.is_match(&f.error_text)
        });
    }

    if failures.is_empty() {
        return None;
    }
    Some(SessionFailures {
        project: info.project.clone(),
        session_id: info.session_id.clone(),
        path: info.path.clone(),
        title: info.title.clone(),
        timestamp: info.timestamp.clone(),
        branch: info.branch.clone(),
        cwd: info.cwd.clone(),
        failures,
    })
}

/// One row of the `stats` table (ADR 0003): a count of Failures sharing the
/// same `(tool, signature)`, where `signature` is the matched [`FAILURE_MARKERS`]
/// entry (or `None` — one no-marker bucket per tool).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureGroup {
    pub tool: Option<String>,
    pub signature: String,
    pub count: usize,
}

/// Fold every Failure in scope into [`FailureGroup`]s keyed by `(tool, signature)`
/// — the aggregate counterpart to [`format_failures`]'s per-Failure list. Sorted
/// by count descending, then tool then signature, so output is deterministic
/// regardless of scan order.
pub fn group_failures(results: &[SessionFailures]) -> Vec<FailureGroup> {
    let mut counts: HashMap<(Option<String>, String), usize> = HashMap::new();
    for session in results {
        for f in &session.failures {
            *counts.entry((f.tool.clone(), failure_signature(&f.error_text))).or_insert(0) += 1;
        }
    }
    let mut groups: Vec<FailureGroup> = counts
        .into_iter()
        .map(|((tool, signature), count)| FailureGroup { tool, signature, count })
        .collect();
    groups.sort_by(|a, b| {
        b.count.cmp(&a.count).then_with(|| a.tool.cmp(&b.tool)).then_with(|| a.signature.cmp(&b.signature))
    });
    groups
}

/// Render Failures with the search index shape: each Session as a header, then
/// each Failure as two lines — `[turn] ✗ <tool>  <command>` and the salient
/// error line (or the whole error text when `full`). At most `max_per_session`
/// per Session (`0` = unlimited), with an actionable `+N more  ›  show` hint.
pub fn format_failures(results: &[SessionFailures], max_per_session: usize, full: bool) -> String {
    if results.is_empty() {
        return "No failures.\n".to_string();
    }
    let mut out = String::new();
    for s in results {
        let short = short_id(&s.session_id);
        out.push_str(&session_header(
            &short,
            // Prefer the real cwd over the mangled directory name (ADR 0005).
            s.cwd.as_deref().unwrap_or(&s.project),
            s.title.as_deref(),
            s.timestamp.as_deref(),
            s.branch.as_deref(),
        ));
        out.push('\n');

        let shown = if max_per_session == 0 {
            s.failures.len()
        } else {
            s.failures.len().min(max_per_session)
        };
        for f in &s.failures[..shown] {
            render_failure(&mut out, f, full);
        }
        let hidden = s.failures.len() - shown;
        if hidden > 0 {
            out.push_str(&format!("  … +{hidden} more  ›  agsearch show {short}\n"));
        }
        out.push('\n');
    }
    out
}

/// Render the `stats` table (ADR 0003 / 0007): one right-aligned `<count>  ✗
/// <tool>  <signature>` row per [`FailureGroup`], biggest first. The signature is
/// a universal marker label or a [`structural_signature`]; `(no marker)` appears
/// only for empty error text. Count-1 rows fold into a `… +N more singleton
/// signatures` trailer (ADR 0008) unless every row is a singleton. Reports
/// cleanly when there are none, like [`format_failures`].
pub fn format_stats(groups: &[FailureGroup]) -> String {
    if groups.is_empty() {
        return "No failures.\n".to_string();
    }
    // Fold count-1 rows into one trailer so the whole-Store table keeps
    // aggregating (ADR 0008) — unless *everything* is a singleton, where
    // folding would hide the entire table behind a bare trailer.
    let recurring: Vec<&FailureGroup> = groups.iter().filter(|g| g.count > 1).collect();
    let (shown, folded) = if recurring.is_empty() {
        (groups.iter().collect::<Vec<_>>(), 0)
    } else {
        (recurring, groups.iter().filter(|g| g.count == 1).count())
    };
    let count_width = shown.iter().map(|g| g.count.to_string().len()).max().unwrap_or(1);
    // Cap the tool column so one long name (e.g. a verbose MCP tool) cannot
    // sparse-out every other row; longer names simply overflow past the pad.
    const TOOL_WIDTH_CAP: usize = 16;
    let tool_width = shown
        .iter()
        .map(|g| g.tool.as_deref().unwrap_or("tool").chars().count())
        .max()
        .unwrap_or(0)
        .min(TOOL_WIDTH_CAP);
    let mut out = String::new();
    for g in shown {
        let tool = g.tool.as_deref().unwrap_or("tool");
        out.push_str(&format!(
            "{:>count_width$}  ✗ {:<tool_width$}  {}\n",
            g.count, tool, g.signature
        ));
    }
    if folded > 0 {
        out.push_str(&format!("… +{folded} more singleton signatures\n"));
    }
    out
}

/// Render one [`Failure`]: a `[turn] ✗ tool  command` line, then its salient
/// error line (with exit code when known) indented beneath — or every line of
/// the error text when `full`.
fn render_failure(out: &mut String, f: &Failure, full: bool) {
    let turn = match f.turn {
        Some(turn) => format!("[{turn}] "),
        None => String::new(),
    };
    let tool = f.tool.as_deref().unwrap_or("tool");
    let command = f.command.as_deref().unwrap_or("");
    let head = format!("  {turn}✗ {tool}  ");
    out.push_str(&head);
    out.push_str(command);
    out.push('\n');

    let indent = " ".repeat(head.chars().count());
    if full {
        for line in clean_error_text(&f.error_text).lines() {
            out.push_str(&indent);
            out.push_str(line);
            out.push('\n');
        }
    } else {
        let prefix = match f.exit_code {
            Some(code) => format!("exit {code} · "),
            None => String::new(),
        };
        out.push_str(&format!("{indent}{prefix}{}\n", salient_line(&f.error_text)));
    }
}

// --- --since: filtering Sessions by recency ------------------------------

/// Days since the Unix epoch for a proleptic-Gregorian date (Howard Hinnant's
/// `days_from_civil`). Hand-rolled to keep `agsearch` free of a date-library
/// dependency — the Store stores ISO 8601, which we otherwise only slice.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse an ISO 8601 timestamp (or bare `YYYY-MM-DD`) to Unix seconds. Time and
/// fractional/zone parts beyond `YYYY-MM-DDThh:mm:ss` are ignored. Returns
/// `None` for anything that is not a well-formed date.
fn parse_iso_to_unix(s: &str) -> Option<i64> {
    if s.len() < 10 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let bytes = s.as_bytes();
    let (hour, min, sec) = if s.len() >= 19 && (bytes[10] == b'T' || bytes[10] == b' ') {
        (s.get(11..13)?.parse().ok()?, s.get(14..16)?.parse().ok()?, s.get(17..19)?.parse().ok()?)
    } else {
        (0, 0, 0)
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + min * 60 + sec)
}

/// Parse a relative duration (`30s`, `1h`, `3d`, `2w`) to seconds, or `None` if
/// it is not a `<number><unit>` with a known unit.
fn parse_duration_secs(value: &str) -> Option<i64> {
    let unit = value.chars().last()?;
    let mult = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        'w' => 604_800,
        _ => return None,
    };
    let n: i64 = value[..value.len() - unit.len_utf8()].parse().ok()?;
    Some(n * mult)
}

/// Resolve a `--since` value to a cutoff in Unix seconds, given `now_unix`. A
/// relative duration (`3d`) is `now - duration`; an absolute ISO date is the
/// date itself. `None` if the value parses as neither.
pub fn since_cutoff(value: &str, now_unix: i64) -> Option<i64> {
    if let Some(secs) = parse_duration_secs(value) {
        return Some(now_unix - secs);
    }
    parse_iso_to_unix(value)
}

/// Whether a Session's recency `timestamp` is at or after `cutoff_unix`. A
/// missing or unparseable timestamp is treated as "too old" (excluded), so
/// `--since` never silently keeps undateable Sessions.
pub fn timestamp_is_since(timestamp: Option<&str>, cutoff_unix: i64) -> bool {
    timestamp.and_then(parse_iso_to_unix).is_some_and(|t| t >= cutoff_unix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn default_matcher_is_literal_and_case_insensitive() {
        let m = Matcher::new("borrow", false, false).unwrap();
        assert!(m.is_match("how do I satisfy the BORROW checker"));
        assert!(m.is_match("borrow"));
        assert!(!m.is_match("unrelated text"));
    }

    #[test]
    fn literal_mode_treats_regex_metacharacters_as_plain_text() {
        let m = Matcher::new("a.c", false, false).unwrap();
        assert!(m.is_match("the literal a.c string"));
        assert!(!m.is_match("abc"), "the dot must not act as a wildcard");
    }

    #[test]
    fn regex_mode_matches_the_query_as_a_pattern() {
        let m = Matcher::new("foo|bar", true, false).unwrap();
        assert!(m.is_match("a bar walked in"));
        assert!(m.is_match("FOO shouted"), "still case-insensitive by default");
        assert!(!m.is_match("neither here"));
    }

    #[test]
    fn case_sensitive_mode_distinguishes_case_in_both_literal_and_regex() {
        let lit = Matcher::new("borrow", false, true).unwrap();
        assert!(lit.is_match("how to borrow safely"));
        assert!(!lit.is_match("the BORROW checker"));

        let re = Matcher::new("Foo|Bar", true, true).unwrap();
        assert!(re.is_match("a Bar appeared"));
        assert!(!re.is_match("a bar appeared"));
    }

    #[test]
    fn an_invalid_regex_query_returns_an_error_instead_of_panicking() {
        let result = Matcher::new("foo(bar", true, false);
        assert!(result.is_err(), "an unbalanced group must be a clean error");
    }

    #[test]
    fn an_invalid_pattern_is_harmless_in_literal_mode() {
        // The same string that is invalid as a regex is fine as a literal,
        // because it is escaped before compilation.
        let m = Matcher::new("foo(bar", false, false).unwrap();
        assert!(m.is_match("got foo(bar here"));
    }

    #[test]
    fn encodes_a_windows_path_the_way_claude_code_stores_it() {
        assert_eq!(
            encode_project_dir(r"E:\projects\rust\agent-conversation-search"),
            "E--projects-rust-agent-conversation-search"
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
    fn all_scope_returns_every_project_dir_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("E--projects-a")).unwrap();
        fs::create_dir(root.join("C--hacking-b")).unwrap();
        fs::write(root.join("loose-file.txt"), "ignored").unwrap();

        let dirs = resolve_scope(root, &Scope::All);

        assert_eq!(dirs, vec![root.join("C--hacking-b"), root.join("E--projects-a")]);
    }

    #[test]
    fn project_scope_matches_name_substring_case_insensitively_and_unions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join("E--projects-games-creature-game")).unwrap();
        fs::create_dir(root.join("E--projects-rust-agsearch")).unwrap();
        fs::create_dir(root.join("C--hacking-jplag")).unwrap();

        let dirs = resolve_scope(root, &Scope::Project { name_substring: "PROJECTS".into() });

        assert_eq!(
            dirs,
            vec![
                root.join("E--projects-games-creature-game"),
                root.join("E--projects-rust-agsearch"),
            ]
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

    /// A default Matcher (literal, case-insensitive) for tests that only care
    /// about which Sessions match, not how the Query is compiled.
    fn lit(query: &str) -> Matcher {
        Matcher::new(query, false, false).unwrap()
    }

    fn write_session(dir: &Path, id: &str, lines: &[&str]) -> PathBuf {
        let path = dir.join(format!("{id}.jsonl"));
        fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    #[test]
    fn search_session_file_scans_just_that_one_session() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        let path = write_session(
            &proj,
            "solo",
            &[r#"{"type":"user","message":{"role":"user","content":"tokio here"}}"#],
        );

        let results = search_session_file(&path, &lit("tokio"), &ContentSet::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].project, "E--projects-demo", "project derived from the parent dir");
        assert_eq!(results[0].session_id, "solo");
    }

    #[test]
    fn a_match_carries_the_turn_number_of_the_message_it_was_found_in() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "turns",
            &[
                r#"{"type":"ai-title","aiTitle":"Tokio chat"}"#,
                r#"{"type":"user","message":{"role":"user","content":"first prompt about tokio"}}"#,
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"a reply"}]}}"#,
                r#"{"type":"user","message":{"role":"user","content":"second tokio question"}}"#,
            ],
        );

        let results = search_project_dirs(&[proj], &lit("tokio"), &ContentSet::default());
        let matches = &results[0].matches;

        // A Title match is metadata, not a turn. The two user prompts are turns
        // 1 and 3 (the assistant reply is turn 2) — the same numbering `show`
        // uses, so a search hit points straight at `show --around <turn>`.
        assert_eq!(matches[0].segment.role, Role::Title);
        assert_eq!(matches[0].turn, None);
        assert_eq!(matches[1].turn, Some(1));
        assert_eq!(matches[2].turn, Some(3));
    }

    #[test]
    fn search_and_show_agree_on_turn_numbers_for_the_same_session() {
        // The handoff invariant (ADR 0002): the turn search prints for a Match
        // must equal the turn `show` numbers that Message. Since ADR 0006 both
        // views derive from one `session::read`, this is now a regression guard
        // on that single numbering rather than a cross-check of two — but the
        // tricky case still matters: a contentless Record (here a signature-only
        // thinking block) must consume a turn number, or every later turn drifts.
        let lines = [
            r#"{"type":"queue-operation","operation":"enqueue"}"#,
            r#"{"type":"user","message":{"role":"user","content":"alpha one"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"","signature":"s"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":"alpha two"}}"#,
        ];

        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(&proj, "agree", &lines);
        let results = search_project_dirs(&[proj], &lit("alpha two"), &ContentSet::default());
        let search_turn = results[0].matches[0].turn;

        let turns = parse_transcript(&lines.join("\n"));
        let show_turn = turns
            .iter()
            .find(|t| t.blocks.iter().any(|b| matches!(b, TurnBlock::Text(s) if s.contains("alpha two"))))
            .map(|t| t.number);

        assert_eq!(search_turn, show_turn, "search and show must agree");
        assert_eq!(search_turn, Some(3), "the contentless thinking Record at turn 2 still counts");
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

        let results =
            search_project_dirs(std::slice::from_ref(&proj), &lit("borrow"), &ContentSet::default());

        assert_eq!(results.len(), 1);
        let s = &results[0];
        assert_eq!(s.project, "E--projects-demo");
        assert_eq!(s.session_id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(s.path, path);
        assert_eq!(s.title.as_deref(), Some("Borrow checker chat"));
        assert_eq!(
            s.matches,
            vec![
                Match { turn: None, segment: Segment { role: Role::Title, text: "Borrow checker chat".into() } },
                Match {
                    turn: Some(1),
                    segment: Segment { role: Role::User, text: "how do I satisfy the BORROW checker".into() },
                },
            ]
        );
    }

    #[test]
    fn list_sessions_lists_every_session_newest_first_without_a_query() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        // An older titled Session, and a newer one — plus a Session with no
        // Message at all (only a noise Record), which search would never surface.
        write_session(
            &proj,
            "aaaaaaaa-0000-0000-0000-000000000000",
            &[
                r#"{"type":"ai-title","aiTitle":"Older chat"}"#,
                r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-01-01T10:00:00.000Z","gitBranch":"main"}"#,
            ],
        );
        write_session(
            &proj,
            "bbbbbbbb-1111-1111-1111-111111111111",
            &[r#"{"type":"user","message":{"role":"user","content":"hi again"},"timestamp":"2026-06-01T10:00:00.000Z"}"#],
        );
        write_session(
            &proj,
            "cccccccc-2222-2222-2222-222222222222",
            &[r#"{"type":"queue-operation","operation":"x","timestamp":"2026-03-01T10:00:00.000Z"}"#],
        );

        let sessions = list_sessions(std::slice::from_ref(&proj));

        // Every Session is listed (the contentless one too — no content filter),
        // newest timestamp first.
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "bbbbbbbb-1111-1111-1111-111111111111", // 2026-06
                "cccccccc-2222-2222-2222-222222222222", // 2026-03 (no Message)
                "aaaaaaaa-0000-0000-0000-000000000000", // 2026-01
            ]
        );
        // Metadata is gathered without a Query.
        assert_eq!(sessions[2].title.as_deref(), Some("Older chat"));
        assert_eq!(sessions[2].branch.as_deref(), Some("main"));
        assert_eq!(sessions[0].title, None);
    }

    #[test]
    fn format_sessions_reuses_the_search_header_and_falls_back_to_untitled() {
        let info = SessionInfo {
            harness: Harness::Claude,
            project: "E--projects-demo".into(),
            session_id: "abcd1234-0000-0000-0000-000000000000".into(),
            path: PathBuf::from("/x/abcd1234-0000-0000-0000-000000000000.jsonl"),
            title: None,
            timestamp: Some("2026-06-01T10:00:00.000Z".into()),
            branch: Some("main".into()),
            cwd: None,
        };
        let out = format_sessions(std::slice::from_ref(&info));
        // Byte-identical to a search Session header (ADR 0004): short-id leads,
        // (untitled) fallback, date sliced to its prefix, branch last.
        assert_eq!(out, "abcd1234 · E--projects-demo · (untitled) · 2026-06-01 · main\n");

        assert_eq!(format_sessions(&[]), "No sessions.\n");
    }

    #[test]
    fn format_sessions_prefers_the_real_cwd_over_the_encoded_directory_name() {
        // The whole reason the tool exists is that the on-disk directory name is
        // mangled (E--projects-demo). When a Session carries its real cwd we show
        // that instead — matching what `projects` already does (lib.rs:466). The
        // unit `--project` matches is unaffected; only the display label changes.
        let info = SessionInfo {
            harness: Harness::Claude,
            project: "E--projects-demo".into(),
            session_id: "abcd1234-0000-0000-0000-000000000000".into(),
            path: PathBuf::from("/x/abcd1234-0000-0000-0000-000000000000.jsonl"),
            title: Some("Demo chat".into()),
            timestamp: Some("2026-06-01T10:00:00.000Z".into()),
            branch: Some("main".into()),
            cwd: Some(r"E:\projects\demo".into()),
        };
        let out = format_sessions(std::slice::from_ref(&info));
        assert_eq!(out, "abcd1234 · E:\\projects\\demo · Demo chat · 2026-06-01 · main\n");
    }

    /// A bare [`SessionInfo`] for a directory, dated and with a cwd — for
    /// exercising the Project fold without touching the filesystem.
    fn sess(dir: &str, id: &str, timestamp: Option<&str>, cwd: Option<&str>) -> SessionInfo {
        SessionInfo {
            harness: Harness::Claude,
            project: dir.into(),
            session_id: id.into(),
            path: PathBuf::from(format!("/store/{dir}/{id}.jsonl")),
            title: None,
            timestamp: timestamp.map(Into::into),
            branch: None,
            cwd: cwd.map(Into::into),
        }
    }

    #[test]
    fn group_projects_folds_case_variant_dirs_and_names_by_newest_cwd() {
        // Two directories differing only by case — the same logical Project on a
        // case-sensitive Store. (This can't be staged on a case-insensitive
        // filesystem, hence constructed data rather than the real Store.)
        let sessions = vec![
            sess("E--projects-demo", "aaaa", Some("2026-01-01T10:00:00.000Z"), Some("E:\\projects\\demo")),
            sess("e--projects-demo", "bbbb", Some("2026-06-01T10:00:00.000Z"), Some("E:\\projects\\demo")),
            sess("E--projects-other", "cccc", Some("2026-06-02T10:00:00.000Z"), Some("E:\\projects\\other")),
        ];

        let projects = group_projects(sessions);

        assert_eq!(projects.len(), 2, "the two case-variant dirs fold into one Project");
        // Newest-touched first.
        assert_eq!(projects[0].name, "E:\\projects\\other");
        assert_eq!(projects[0].session_count, 1);
        // The folded Project: count summed, last-touched = the newer Session,
        // name from the newest Session's cwd.
        assert_eq!(projects[1].name, "E:\\projects\\demo");
        assert_eq!(projects[1].session_count, 2);
        assert_eq!(projects[1].last_touched.as_deref(), Some("2026-06-01T10:00:00.000Z"));
    }

    #[test]
    fn group_projects_falls_back_to_the_encoded_name_without_a_cwd() {
        let projects = group_projects(vec![sess("E--projects-x", "aaaa", Some("2026-06-01T10:00:00.000Z"), None)]);
        assert_eq!(projects[0].name, "E--projects-x");
    }

    #[test]
    fn format_projects_renders_rows_and_pluralises() {
        let projects = vec![
            ProjectInfo {
                name: "E:\\projects\\rust".into(),
                session_count: 7,
                last_touched: Some("2026-06-02T10:00:00.000Z".into()),
            },
            ProjectInfo { name: "E:\\projects\\solo".into(), session_count: 1, last_touched: None },
        ];
        let out = format_projects(&projects);
        assert_eq!(
            out,
            "E:\\projects\\rust · 7 sessions · 2026-06-02\nE:\\projects\\solo · 1 session\n"
        );
        assert_eq!(format_projects(&[]), "No projects.\n");
    }

    /// Build a [`SessionMatches`] from bare Segments, assigning each a 1-based
    /// turn for tests that only care about rendering, not turn alignment.
    fn session(project: &str, title: Option<&str>, segments: Vec<Segment>) -> SessionMatches {
        let matches = segments
            .into_iter()
            .enumerate()
            .map(|(i, segment)| Match { turn: Some(i + 1), segment })
            .collect();
        SessionMatches {
            harness: Harness::Claude,
            project: project.into(),
            session_id: "11111111-2222-3333-4444-555555555555".into(),
            path: PathBuf::from("/x/11111111-2222-3333-4444-555555555555.jsonl"),
            title: title.map(Into::into),
            timestamp: None,
            branch: None,
            cwd: None,
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

        let out = format_results(&results, &lit("borrow"), 0, false);

        assert!(out.contains("E--projects-demo"), "header shows project: {out}");
        assert!(out.contains("Borrow checker chat"), "header shows title: {out}");
        assert!(
            out.contains("how do I satisfy the BORROW checker"),
            "match text shown: {out}"
        );
        assert!(out.to_lowercase().contains("user"), "match labelled by role: {out}");
    }

    #[test]
    fn search_header_prefers_the_real_cwd_over_the_encoded_directory_name() {
        // search's default-verb header must recover the real path too — not just
        // `sessions`/`projects` (issue 18). The encoded name stays the fallback.
        let mut s = session("E--projects-demo", Some("Borrow chat"), vec![
            Segment { role: Role::User, text: "borrow".into() },
        ]);
        s.cwd = Some(r"E:\projects\demo".into());

        let out = format_results(&[s], &lit("borrow"), 0, false);

        assert!(out.contains(r"E:\projects\demo"), "header shows real cwd: {out}");
        assert!(!out.contains("E--projects-demo"), "not the mangled name: {out}");
    }

    #[test]
    fn header_shows_the_date_sliced_from_the_timestamp() {
        let mut s = session("E--projects-demo", Some("Borrow chat"), vec![
            Segment { role: Role::User, text: "borrow".into() },
        ]);
        s.timestamp = Some("2026-06-01T10:00:00.000Z".into());

        let out = format_results(&[s], &lit("borrow"), 0, false);

        assert!(out.contains("2026-06-01"), "header shows YYYY-MM-DD date: {out}");
        assert!(!out.contains("10:00:00"), "but not the time component: {out}");
    }

    #[test]
    fn header_shows_the_git_branch() {
        let mut s = session("E--projects-demo", Some("Borrow chat"), vec![
            Segment { role: Role::User, text: "borrow".into() },
        ]);
        s.timestamp = Some("2026-06-01T10:00:00.000Z".into());
        s.branch = Some("feature/search".into());

        let out = format_results(&[s], &lit("borrow"), 0, false);

        assert!(out.contains("feature/search"), "header shows branch: {out}");
    }

    #[test]
    fn header_leads_with_the_short_session_id() {
        let s = session(
            "E--projects-demo",
            Some("Borrow chat"),
            vec![Segment { role: Role::User, text: "borrow".into() }],
        );

        let out = format_results(&[s], &lit("borrow"), 0, false);

        // The helper's session_id is 11111111-2222-… so the short id is 11111111.
        assert!(out.lines().next().unwrap().starts_with("11111111"), "header leads with short id: {out}");
    }

    #[test]
    fn each_match_line_is_prefixed_with_its_turn_number() {
        let s = session(
            "p",
            Some("t"),
            vec![
                Segment { role: Role::User, text: "alpha".into() },
                Segment { role: Role::Assistant, text: "alpha beta".into() },
            ],
        );

        let out = format_results(&[s], &lit("alpha"), 0, false);

        assert!(out.contains("[1] user:"), "first match shows its turn: {out}");
        assert!(out.contains("[2] assistant:"), "second match shows its turn: {out}");
    }

    #[test]
    fn a_title_match_shows_no_turn_bracket() {
        let s = SessionMatches {
            harness: Harness::Claude,
            project: "p".into(),
            session_id: "abcd1234-rest".into(),
            path: PathBuf::from("/x/abcd1234-rest.jsonl"),
            title: Some("Borrow chat".into()),
            timestamp: None,
            branch: None,
            cwd: None,
            matches: vec![Match {
                turn: None,
                segment: Segment { role: Role::Title, text: "Borrow chat".into() },
            }],
        };

        let out = format_results(&[s], &lit("borrow"), 0, false);

        assert!(out.contains("  title: Borrow chat"), "title rendered without a turn bracket: {out}");
    }

    #[test]
    fn snippet_is_centered_on_the_match_with_ellipses_when_cut() {
        let text = format!("{}NEEDLE {}", "alpha ".repeat(60), "omega ".repeat(60));
        let s = session("p", Some("t"), vec![Segment { role: Role::User, text }]);

        let out = format_results(&[s], &lit("NEEDLE"), 0, false);
        // The match line is the indented one carrying NEEDLE.
        let line = out.lines().find(|l| l.contains("NEEDLE")).expect("a line with the match");

        assert!(line.contains('…'), "ellipsis marks the cut: {line}");
        assert!(line.contains("alpha"), "context before the match is shown: {line}");
        assert!(line.contains("omega"), "context after the match is shown: {line}");
        assert!(
            line.chars().count() <= SNIPPET_MAX_CHARS + 30,
            "the window is bounded (got {} chars): {line}",
            line.chars().count()
        );
    }

    #[test]
    fn snippet_at_the_start_has_no_leading_ellipsis() {
        let text = format!("NEEDLE {}", "omega ".repeat(100));
        let s = session("p", Some("t"), vec![Segment { role: Role::User, text }]);

        let out = format_results(&[s], &lit("NEEDLE"), 0, false);
        let snippet = out.lines().find(|l| l.contains("NEEDLE")).unwrap().trim_start();

        assert!(snippet.starts_with("[1] user: NEEDLE"), "no leading ellipsis at the start: {snippet}");
        assert!(snippet.ends_with('…'), "trailing ellipsis where cut: {snippet}");
    }

    #[test]
    fn collapses_multiline_match_text_onto_one_line() {
        let results = vec![session(
            "p",
            Some("t"),
            vec![Segment { role: Role::User, text: "line one\n\n   line two".into() }],
        )];

        let out = format_results(&results, &lit("line"), 0, false);

        assert!(out.contains("line one line two"), "collapsed: {out}");
        assert!(!out.contains("line one\n"), "no embedded newline in snippet: {out}");
    }

    #[test]
    fn caps_matches_per_session_and_notes_how_many_more() {
        let s = session("p", Some("t"), vec![
            Segment { role: Role::User, text: "match-one".into() },
            Segment { role: Role::User, text: "match-two".into() },
            Segment { role: Role::User, text: "match-three".into() },
            Segment { role: Role::User, text: "match-four".into() },
            Segment { role: Role::User, text: "match-five".into() },
        ]);

        let out = format_results(&[s], &lit("match"), 3, false);

        assert!(out.contains("match-one") && out.contains("match-three"), "first 3 shown: {out}");
        assert!(!out.contains("match-four") && !out.contains("match-five"), "rest hidden: {out}");
        assert!(out.contains("+2 more"), "notes how many were hidden: {out}");
        assert!(
            out.contains("agsearch show 11111111"),
            "overflow line is an actionable show hint: {out}"
        );
    }

    #[test]
    fn a_cap_of_zero_means_unlimited() {
        let s = session("p", Some("t"), vec![
            Segment { role: Role::User, text: "match-one".into() },
            Segment { role: Role::User, text: "match-two".into() },
            Segment { role: Role::User, text: "match-three".into() },
            Segment { role: Role::User, text: "match-four".into() },
        ]);

        let out = format_results(&[s], &lit("match"), 0, false);

        assert!(out.contains("match-four"), "no cap applied: {out}");
        assert!(!out.contains("more"), "no '+N more' line: {out}");
    }

    #[test]
    fn files_mode_prints_one_session_path_per_line() {
        let mut a = session("p", Some("t"), vec![Segment { role: Role::User, text: "x".into() }]);
        a.path = PathBuf::from("/store/proj/aaa.jsonl");
        let mut b = session("p", Some("t"), vec![Segment { role: Role::User, text: "x".into() }]);
        b.path = PathBuf::from("/store/proj/bbb.jsonl");

        let out = format_paths(&[a, b]);

        assert_eq!(out, "/store/proj/aaa.jsonl\n/store/proj/bbb.jsonl\n");
    }

    #[test]
    fn color_on_highlights_the_match_with_ansi_codes() {
        let s = session("p", Some("t"), vec![
            Segment { role: Role::User, text: "the BORROW checker".into() },
        ]);

        let out = format_results(&[s], &lit("borrow"), 0, true);

        assert!(out.contains('\u{1b}'), "ANSI escape present when colour is on: {out:?}");
        assert!(out.contains("checker"), "surrounding text still present: {out:?}");
    }

    #[test]
    fn color_off_emits_no_ansi_codes() {
        let s = session("p", Some("t"), vec![
            Segment { role: Role::User, text: "the BORROW checker".into() },
        ]);

        let out = format_results(&[s], &lit("borrow"), 0, false);

        assert!(!out.contains('\u{1b}'), "no ANSI when colour is off: {out:?}");
    }

    #[test]
    fn renders_a_clear_message_when_there_are_no_matches() {
        let out = format_results(&[], &lit("anything"), 0, false);
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
    fn deduplicates_when_the_same_project_dir_is_passed_more_than_once() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "only",
            &[r#"{"type":"user","message":{"role":"user","content":"alpha match"}}"#],
        );

        let results =
            search_project_dirs(&[proj.clone(), proj.clone()], &lit("alpha"), &ContentSet::default());

        assert_eq!(results.len(), 1);
    }

    #[test]
    fn returns_sessions_in_deterministic_order_despite_parallel_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        for id in ["33-c", "11-a", "22-b"] {
            write_session(
                &proj,
                id,
                &[r#"{"type":"user","message":{"role":"user","content":"alpha match"}}"#],
            );
        }

        let results = search_project_dirs(&[proj], &lit("alpha"), &ContentSet::default());

        let ids: Vec<_> = results.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids, vec!["11-a", "22-b", "33-c"]);
    }

    #[test]
    fn search_honours_the_matcher_case_sensitivity() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "shouting",
            &[r#"{"type":"user","message":{"role":"user","content":"the BORROW checker"}}"#],
        );

        let case_sensitive = Matcher::new("borrow", false, true).unwrap();
        assert!(search_project_dirs(
            std::slice::from_ref(&proj),
            &case_sensitive,
            &ContentSet::default()
        )
        .is_empty());

        let case_insensitive = Matcher::new("borrow", false, false).unwrap();
        assert_eq!(
            search_project_dirs(&[proj], &case_insensitive, &ContentSet::default()).len(),
            1
        );
    }

    #[test]
    fn sessions_are_returned_newest_first_by_record_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        // IDs are chosen so alphabetical order (the old path sort) is the
        // OPPOSITE of recency order, so a path-sorted result would fail.
        write_session(
            &proj,
            "01-alphabetically-first",
            &[r#"{"type":"user","message":{"role":"user","content":"alpha match"},"timestamp":"2026-01-15T10:00:00.000Z"}"#],
        );
        write_session(
            &proj,
            "99-alphabetically-last",
            &[r#"{"type":"user","message":{"role":"user","content":"alpha match"},"timestamp":"2026-06-01T10:00:00.000Z"}"#],
        );

        let results = search_project_dirs(&[proj], &lit("alpha"), &ContentSet::default());

        let ids: Vec<_> = results.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids, vec!["99-alphabetically-last", "01-alphabetically-first"]);
    }

    #[test]
    fn search_captures_the_git_branch_from_records() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "branchy",
            &[r#"{"type":"user","message":{"role":"user","content":"alpha match"},"gitBranch":"feature/search"}"#],
        );

        let results = search_project_dirs(&[proj], &lit("alpha"), &ContentSet::default());

        assert_eq!(results[0].branch.as_deref(), Some("feature/search"));
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

        let results = search_project_dirs(&[proj], &lit("tokio"), &ContentSet::default());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "hit");
    }

    #[test]
    fn a_query_only_present_in_thinking_is_excluded_by_default_but_found_with_thinking() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "only-thinking",
            &[r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"the secret password is hunter2"}]}}"#],
        );

        let default = ContentSet::default();
        assert!(
            search_project_dirs(std::slice::from_ref(&proj), &lit("hunter2"), &default).is_empty(),
            "thinking is not in the default content set"
        );

        let with_thinking = ContentSet { thinking: true, ..ContentSet::default() };
        assert_eq!(
            search_project_dirs(&[proj], &lit("hunter2"), &with_thinking).len(),
            1,
            "--thinking includes thinking blocks"
        );
    }

    #[test]
    fn tool_calls_and_results_are_excluded_by_default_but_found_with_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "with-tools",
            &[
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo zzztest"}}]}}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"zzztest output here"}]}}"#,
            ],
        );

        let default = ContentSet::default();
        assert!(
            search_project_dirs(std::slice::from_ref(&proj), &lit("zzztest"), &default).is_empty(),
            "tool calls/results are not in the default content set"
        );

        let with_tools = ContentSet { tools: true, ..ContentSet::default() };
        let results = search_project_dirs(&[proj], &lit("zzztest"), &with_tools);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 2, "both the tool_use and tool_result match");
        assert!(results[0].matches.iter().all(|m| m.segment.role == Role::Tool));
    }

    // --- failure salient-line picker -------------------------------------

    #[test]
    fn salient_line_prefers_a_compiler_error_over_the_useless_exit_code_line() {
        // The handoff's load-bearing finding: the first line is just the exit
        // code; the real error is several lines down.
        let text = "Exit code 101\n   Compiling x\nerror[E0433]: failed to resolve: use of undeclared crate\n  --> src/main.rs:3:5";
        assert_eq!(salient_line(text), "error[E0433]: failed to resolve: use of undeclared crate");
    }

    #[test]
    fn salient_line_finds_a_panic() {
        let text = "Exit code 101\nrunning 1 test\nthread 'main' panicked at src/lib.rs:5:9:\nassertion `left == right` failed";
        // `panicked at` outranks `assertion failed` in the priority list.
        assert_eq!(salient_line(text), "thread 'main' panicked at src/lib.rs:5:9:");
    }

    #[test]
    fn salient_line_finds_a_bash_eof() {
        let text = "Exit code 2\n/usr/bin/bash: eval: line 1: unexpected EOF while looking for matching `\"'";
        assert_eq!(
            salient_line(text),
            "/usr/bin/bash: eval: line 1: unexpected EOF while looking for matching `\"'"
        );
    }

    #[test]
    fn salient_line_finds_a_missing_file() {
        let text = "File does not exist: /tmp/nope.txt";
        assert_eq!(salient_line(text), "File does not exist: /tmp/nope.txt");
    }

    #[test]
    fn salient_line_falls_back_to_the_last_non_empty_line() {
        let text = "some output\nwith no recognised marker\nfinal line\n\n";
        assert_eq!(salient_line(text), "final line");
    }

    #[test]
    fn salient_line_strips_ansi_colour_codes() {
        let text = "\u{1b}[31merror[E0382]: borrow of moved value\u{1b}[0m";
        assert_eq!(salient_line(text), "error[E0382]: borrow of moved value");
    }

    #[test]
    fn exit_code_is_parsed_when_present_and_absent_otherwise() {
        assert_eq!(exit_code("Exit code 101\nerror[E0433]"), Some(101));
        assert_eq!(exit_code("File does not exist: /x"), None);
    }

    // --- failure signature: the grouping key (ADR 0003) ------------------

    #[test]
    fn failure_signature_of_a_language_error_is_its_structural_shape_not_a_hardcoded_label() {
        // `error[` is a display hint only: salient_line still picks it over the
        // earlier `error:` line, but the bucket is the masked shape of that line,
        // so the table never hard-codes Rust's syntax as a universal signature.
        let text = "Exit code 101\nerror: aborting\nerror[E0433]: cannot find type";
        assert_eq!(failure_signature(text), "error[EN]: cannot find type");
    }

    #[test]
    fn failure_signature_falls_back_to_a_structural_shape_when_no_marker_matches() {
        // No marker, so the (masked) salient line becomes the bucket — not one
        // giant `(no marker)` lump. `(no marker)` survives only for empty text.
        assert_eq!(
            failure_signature("just some unremarkable stdout\nwith nothing notable"),
            "with nothing notable",
        );
        assert_eq!(failure_signature(""), "(no marker)");
    }

    #[test]
    fn structural_signature_masks_paths_and_numbers_so_like_errors_share_a_shape() {
        assert_eq!(
            structural_signature("thread 'main' panicked at src/lib.rs:5:9:"),
            "thread 'main' panicked at <path>",
        );
        // Two panics at different sites collapse to one shape.
        assert_eq!(
            structural_signature("thread 'main' panicked at src/main.rs:42:1:"),
            "thread 'main' panicked at <path>",
        );
    }

    #[test]
    fn failure_signature_and_salient_line_agree_on_the_line() {
        // Display and grouping still look at the *same* line; the table just
        // masks it. A `panicked at` hint highlights the panic and buckets its shape.
        let text = "Exit code 101\nrunning 1 test\nthread 'main' panicked at src/lib.rs:5:9:";
        assert_eq!(failure_signature(text), "thread 'main' panicked at <path>");
        assert!(salient_line(text).contains("panicked at"));
    }

    #[test]
    fn failure_signature_unwraps_tool_use_error_and_classifies_the_inner_message() {
        // The wrapper must not defeat classification (issue 14).
        let text = "<tool_use_error>String to replace not found in file.</tool_use_error>";
        assert_eq!(failure_signature(text), "String to replace not found");
    }

    #[test]
    fn failure_signature_classifies_the_common_universal_tool_errors() {
        assert_eq!(
            failure_signature("File has not been read yet. Read it first before writing to it."),
            "has not been read",
        );
        assert_eq!(
            failure_signature("File content (38631 tokens) exceeds maximum allowed tokens (25000)."),
            "exceeds maximum allowed tokens",
        );
        assert_eq!(
            failure_signature("foo : The term 'foo' is not recognized as the name of a cmdlet"),
            "is not recognized",
        );
        assert_eq!(
            failure_signature("<tool_use_error>Blocked: sleep 90 followed by: cat x</tool_use_error>"),
            "Blocked",
        );
    }

    #[test]
    fn failure_signature_buckets_harness_noise_under_clean_labels() {
        // Structurally is_error, but the tool did not error — bucketed, not
        // filtered, under a short label distinct from the matched substring.
        let rejected = "The user doesn't want to proceed with this tool use. The tool use was rejected";
        assert_eq!(failure_signature(rejected), "rejected");
        let unavailable = "claude-opus-4-8 is temporarily unavailable, so auto mode cannot determine safety";
        assert_eq!(failure_signature(unavailable), "unavailable");
        // A parallel-batch sibling errored, so this call was cancelled unrun.
        let cancelled = "Cancelled: parallel tool call Bash(cd \"D:\\x\" && git status) errored";
        assert_eq!(failure_signature(cancelled), "cancelled");
    }

    #[test]
    fn salient_line_unwraps_the_tool_use_error_wrapper() {
        let text = "<tool_use_error>String to replace not found in file.</tool_use_error>";
        assert_eq!(salient_line(text), "String to replace not found in file.");
    }

    // --- finding Failures by structure -----------------------------------

    #[test]
    fn finds_a_failure_joining_tool_result_to_its_tool_use() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "fails",
            &[
                r#"{"type":"user","message":{"role":"user","content":"go"}}"#,
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"Exit code 101\nerror[E0433]: failed to resolve"}]}}"#,
            ],
        );

        let results = failed_in_project_dirs(&[proj], None);

        assert_eq!(results.len(), 1);
        let failures = &results[0].failures;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].tool.as_deref(), Some("Bash"), "tool joined from tool_use");
        assert_eq!(failures[0].command.as_deref(), Some("cargo test"), "command joined from tool_use");
        assert_eq!(failures[0].exit_code, Some(101));
        assert_eq!(failures[0].turn, Some(3), "turn 3: user(1), assistant(2), tool_result(3)");
    }

    #[test]
    fn sessions_without_a_failure_are_omitted() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "ok",
            &[r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"all good"}]}}"#],
        );

        assert!(failed_in_project_dirs(&[proj], None).is_empty(), "a successful tool_result is not a Failure");
    }

    #[test]
    fn a_query_filters_failures_by_command_or_error_text() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("E--projects-demo");
        fs::create_dir(&proj).unwrap();
        write_session(
            &proj,
            "two",
            &[
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"Bash","input":{"command":"cargo test"}}]}}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","is_error":true,"content":"Exit code 101"}]}}"#,
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"b","name":"Bash","input":{"command":"npm run build"}}]}}"#,
                r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"b","is_error":true,"content":"webpack failed"}]}}"#,
            ],
        );

        let all = failed_in_project_dirs(std::slice::from_ref(&proj), None);
        assert_eq!(all[0].failures.len(), 2, "no query lists every Failure");

        let cargo = failed_in_project_dirs(&[proj], Some(&lit("cargo")));
        assert_eq!(cargo[0].failures.len(), 1, "query keeps only the matching command");
        assert_eq!(cargo[0].failures[0].command.as_deref(), Some("cargo test"));
    }

    // --- aggregating Failures into a stats table (ADR 0003) --------------

    fn failure(tool: &str, error_text: &str) -> Failure {
        Failure {
            turn: None,
            tool: Some(tool.into()),
            command: None,
            exit_code: exit_code(error_text),
            error_text: error_text.into(),
        }
    }

    fn session_of(failures: Vec<Failure>) -> SessionFailures {
        SessionFailures {
            project: "E--projects-demo".into(),
            session_id: "abcd1234-rest".into(),
            path: PathBuf::from("/x/abcd1234-rest.jsonl"),
            title: None,
            timestamp: None,
            branch: None,
            cwd: None,
            failures,
        }
    }

    #[test]
    fn group_failures_collapses_same_shape_errors_and_labels_universal_ones() {
        let s = session_of(vec![
            // Two compile errors of the *same shape* (only the masked code
            // differs) collapse into one structural group of 2.
            failure("PowerShell", "Exit code 101\nerror[E0433]: cannot find type"),
            failure("PowerShell", "Exit code 101\nerror[E0609]: cannot find type"),
            // A universal OS error keeps its stable label.
            failure("Bash", "Exit code 2\nunexpected EOF while looking for matching `\"'"),
        ]);

        let groups = group_failures(std::slice::from_ref(&s));

        let ps = groups
            .iter()
            .find(|g| g.tool.as_deref() == Some("PowerShell") && g.signature == "error[EN]: cannot find type")
            .expect("a PowerShell structural group");
        assert_eq!(ps.count, 2);
        let bash = groups
            .iter()
            .find(|g| g.tool.as_deref() == Some("Bash") && g.signature == "unexpected EOF")
            .expect("a Bash unexpected-EOF group");
        assert_eq!(bash.count, 1);
    }

    #[test]
    fn group_failures_splits_differently_shaped_errors() {
        // The flip side of the contract: two `error[` lines whose *messages*
        // differ are genuinely different problems, so they no longer share a bucket.
        let s = session_of(vec![
            failure("PowerShell", "error[E0433]: cannot find type"),
            failure("PowerShell", "error[E0382]: borrow of moved value"),
        ]);

        let groups = group_failures(&[s]);

        assert_eq!(groups.len(), 2, "different messages are different buckets");
        assert!(groups.iter().all(|g| g.count == 1));
    }

    #[test]
    fn group_failures_buckets_unmatched_failures_by_structural_shape() {
        let s = session_of(vec![
            failure("Bash", "some unremarkable output\nnothing notable here"),
            failure("Glob", "another line the markers do not recognise"),
        ]);

        let groups = group_failures(&[s]);

        // No marker matched, yet nothing lands in one `(no marker)` lump — each
        // gets the masked shape of its salient line.
        assert!(groups.iter().all(|g| g.signature != "(no marker)"), "no lumping");
        assert!(groups.iter().any(|g| g.signature == "nothing notable here"));
        assert!(groups.iter().any(|g| g.signature == "another line the markers do not recognise"));
        assert_eq!(groups.iter().map(|g| g.count).sum::<usize>(), 2);
    }

    #[test]
    fn group_failures_buckets_harness_noise_without_filtering_it() {
        let s = session_of(vec![
            failure("Bash", "Exit code 2\nThe tool use was rejected"),
            failure("PowerShell", "claude is temporarily unavailable"),
            failure("PowerShell", "Exit code 101\nerror[E0433]: cannot find type"),
        ]);

        let groups = group_failures(&[s]);

        // All three remain present (nothing filtered); noise carries clean labels.
        assert_eq!(groups.iter().map(|g| g.count).sum::<usize>(), 3, "no Failure is dropped");
        assert!(groups.iter().any(|g| g.signature == "rejected"), "rejection bucketed");
        assert!(groups.iter().any(|g| g.signature == "unavailable"), "unavailability bucketed");
    }

    #[test]
    fn group_failures_sorts_by_count_descending() {
        let s = session_of(vec![
            failure("Bash", "fatal: not a git repository"),
            failure("PowerShell", "error[E0433]: mismatched types"),
            failure("PowerShell", "error[E0609]: mismatched types"),
            failure("PowerShell", "error[E0425]: mismatched types"),
        ]);

        let groups = group_failures(&[s]);

        // The three same-shape compile errors are the biggest group.
        assert_eq!(groups.first().unwrap().count, 3, "the biggest group leads");
        assert_eq!(groups.first().unwrap().signature, "error[EN]: mismatched types");
    }

    // --- rendering Failures ----------------------------------------------

    fn one_failure(failure: Failure) -> SessionFailures {
        SessionFailures {
            project: "E--projects-demo".into(),
            session_id: "abcd1234-rest".into(),
            path: PathBuf::from("/x/abcd1234-rest.jsonl"),
            title: Some("Build chat".into()),
            timestamp: None,
            branch: None,
            cwd: None,
            failures: vec![failure],
        }
    }

    #[test]
    fn format_failures_renders_id_turn_tool_command_and_salient_line() {
        let s = one_failure(Failure {
            turn: Some(3),
            tool: Some("Bash".into()),
            command: Some("cargo test".into()),
            exit_code: Some(101),
            error_text: "Exit code 101\nerror[E0433]: failed to resolve".into(),
        });

        let out = format_failures(&[s], 3, false);

        assert!(out.starts_with("abcd1234"), "header leads with the short id: {out}");
        assert!(out.contains("[3] ✗ Bash"), "turn + failed tool: {out}");
        assert!(out.contains("cargo test"), "command shown: {out}");
        assert!(out.contains("exit 101 · error[E0433]: failed to resolve"), "salient line with exit: {out}");
    }

    #[test]
    fn format_failures_header_prefers_the_real_cwd_over_the_encoded_name() {
        let mut s = one_failure(Failure {
            turn: Some(3),
            tool: Some("Bash".into()),
            command: Some("cargo test".into()),
            exit_code: Some(101),
            error_text: "Exit code 101\nerror[E0433]: failed".into(),
        });
        s.cwd = Some(r"E:\projects\demo".into());

        let out = format_failures(&[s], 3, false);

        assert!(out.contains(r"E:\projects\demo"), "header shows real cwd: {out}");
        assert!(!out.contains("E--projects-demo"), "not the mangled name: {out}");
    }

    #[test]
    fn format_failures_full_shows_the_entire_error_text() {
        let s = one_failure(Failure {
            turn: Some(3),
            tool: Some("Bash".into()),
            command: Some("cargo test".into()),
            exit_code: Some(101),
            error_text: "Exit code 101\nline two\nerror[E0433]: failed".into(),
        });

        let out = format_failures(&[s], 3, true);

        assert!(out.contains("line two"), "--full shows non-salient lines too: {out}");
        assert!(out.contains("error[E0433]: failed"), "and the salient one: {out}");
    }

    #[test]
    fn format_failures_reports_cleanly_when_there_are_none() {
        assert!(format_failures(&[], 3, false).to_lowercase().contains("no failures"));
    }

    #[test]
    fn format_stats_renders_a_counts_table_of_tool_signature_and_count() {
        let groups = vec![
            FailureGroup { tool: Some("PowerShell".into()), signature: "error[EN]: x".into(), count: 12 },
            FailureGroup { tool: Some("Bash".into()), signature: "unexpected EOF".into(), count: 4 },
        ];

        let out = format_stats(&groups);

        assert!(out.contains("12"), "the count: {out}");
        assert!(out.contains("✗ PowerShell"), "tool with the failed glyph: {out}");
        assert!(out.contains("error[EN]: x"), "the structural signature: {out}");
        assert!(out.contains("unexpected EOF"));
        // Counts are right-aligned to a common width, so 4 trails 12.
        let twelve = out.find("12").unwrap();
        let four = out.find(" 4").unwrap();
        assert!(twelve < four, "biggest count first: {out}");
    }

    #[test]
    fn format_stats_renders_the_no_marker_bucket() {
        let groups = vec![FailureGroup { tool: Some("Read".into()), signature: "(no marker)".into(), count: 5 }];
        assert!(format_stats(&groups).contains("(no marker)"), "names the empty-text bucket");
    }

    #[test]
    fn format_stats_caps_the_tool_column_so_a_long_name_does_not_sparse_out_rows() {
        let groups = vec![
            FailureGroup { tool: Some("PowerShell".into()), signature: "error[EN]: x".into(), count: 2 },
            FailureGroup {
                tool: Some("mcp__ccd_session_mgmt__search_session_transcripts".into()),
                signature: "(no marker)".into(),
                // count 2 so the row is not folded away as a singleton.
                count: 2,
            },
        ];

        let out = format_stats(&groups);
        let short_row = out.lines().find(|l| l.contains("PowerShell")).unwrap();
        assert!(
            short_row.chars().count() < 40,
            "the long MCP name must not pad the PowerShell row out: {short_row:?}"
        );
    }

    #[test]
    fn format_stats_folds_singletons_into_a_trailer_when_recurring_rows_exist() {
        let groups = vec![
            FailureGroup { tool: Some("Edit".into()), signature: "has not been read".into(), count: 44 },
            FailureGroup { tool: Some("Bash".into()), signature: "=== user settings ===".into(), count: 1 },
            FailureGroup { tool: Some("Bash".into()), signature: "---README---".into(), count: 1 },
        ];

        let out = format_stats(&groups);

        assert!(out.contains("has not been read"), "recurring rows stay: {out}");
        assert!(!out.contains("=== user settings ==="), "singletons fold away: {out}");
        assert!(!out.contains("---README---"), "singletons fold away: {out}");
        assert!(out.contains("+2 more singleton signatures"), "the trailer counts them: {out}");
    }

    #[test]
    fn format_stats_has_no_trailer_when_there_are_no_singletons() {
        let groups =
            vec![FailureGroup { tool: Some("Edit".into()), signature: "has not been read".into(), count: 2 }];
        assert!(!format_stats(&groups).contains("more singleton"), "no trailer when N=0");
    }

    #[test]
    fn format_stats_shows_singletons_when_every_row_is_one() {
        // An all-singleton table has no noise burying signal — folding it would
        // hide everything behind a bare trailer.
        let groups =
            vec![FailureGroup { tool: Some("Read".into()), signature: "does not exist".into(), count: 1 }];
        let out = format_stats(&groups);
        assert!(out.contains("does not exist"), "shown, not folded: {out}");
        assert!(!out.contains("more singleton"), "{out}");
    }

    #[test]
    fn format_stats_reports_cleanly_when_there_are_none() {
        assert!(format_stats(&[]).to_lowercase().contains("no failures"));
    }

    // --- --since recency filter ------------------------------------------

    #[test]
    fn parse_iso_to_unix_anchors_on_the_epoch() {
        assert_eq!(parse_iso_to_unix("1970-01-01"), Some(0));
        assert_eq!(parse_iso_to_unix("1970-01-02"), Some(86_400));
        assert_eq!(parse_iso_to_unix("1970-01-01T01:00:00.000Z"), Some(3_600));
    }

    #[test]
    fn since_parses_relative_durations_against_now() {
        let now = 1_000_000;
        assert_eq!(since_cutoff("1h", now), Some(now - 3_600));
        assert_eq!(since_cutoff("3d", now), Some(now - 3 * 86_400));
        assert_eq!(since_cutoff("2w", now), Some(now - 14 * 86_400));
    }

    #[test]
    fn since_parses_an_absolute_iso_date() {
        // The absolute form ignores `now`; the cutoff is the date itself.
        assert_eq!(since_cutoff("2026-05-01", 999), parse_iso_to_unix("2026-05-01"));
    }

    #[test]
    fn since_rejects_unparseable_values() {
        assert_eq!(since_cutoff("yesterday", 0), None);
        assert_eq!(since_cutoff("3x", 0), None);
    }

    #[test]
    fn timestamp_is_since_excludes_older_missing_and_unparseable() {
        let cutoff = parse_iso_to_unix("2026-05-01").unwrap();
        assert!(timestamp_is_since(Some("2026-06-01T10:00:00.000Z"), cutoff), "newer kept");
        assert!(!timestamp_is_since(Some("2026-04-01T10:00:00.000Z"), cutoff), "older excluded");
        assert!(!timestamp_is_since(None, cutoff), "missing timestamp excluded");
        assert!(!timestamp_is_since(Some("not a date"), cutoff), "unparseable excluded");
    }

    // --- session-id prefix resolution -----------------------------------

    /// Plant an (empty-content) Session file under a Project so its stem is
    /// discoverable by prefix resolution.
    fn plant(projects_root: &Path, project: &str, session_id: &str) -> PathBuf {
        let dir = projects_root.join(project);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{session_id}.jsonl"));
        fs::write(&path, "").unwrap();
        path
    }

    #[test]
    fn a_unique_prefix_resolves_to_one_session_across_the_whole_store() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let target = plant(root, "E--projects-a", "4c28878f-c921-4892-8d26-78df5801f301");
        // A Session in a *different* Project — prefix resolution spans the Store.
        plant(root, "C--hacking-b", "9999aaaa-0000-0000-0000-000000000000");

        assert_eq!(
            resolve_session_prefix(root, "4c28878f"),
            SessionRef::Unique(target)
        );
    }

    #[test]
    fn an_ambiguous_prefix_lists_every_matching_session_id_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        plant(root, "E--projects-a", "abc222-second");
        plant(root, "E--projects-a", "abc111-first");

        assert_eq!(
            resolve_session_prefix(root, "abc"),
            SessionRef::Ambiguous(vec!["abc111-first".into(), "abc222-second".into()])
        );
    }

    #[test]
    fn a_prefix_matching_nothing_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        plant(root, "E--projects-a", "abc111-first");

        assert_eq!(resolve_session_prefix(root, "zzz"), SessionRef::NotFound);
    }

    #[test]
    fn an_exact_full_id_wins_over_a_longer_session_that_shares_it() {
        // A complete session-id must never be reported ambiguous just because a
        // longer id starts with the same characters.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let exact = plant(root, "E--projects-a", "abc111");
        plant(root, "E--projects-a", "abc111-longer");

        assert_eq!(resolve_session_prefix(root, "abc111"), SessionRef::Unique(exact));
    }

    // --- transcript parsing + rendering ----------------------------------

    #[test]
    fn parses_messages_into_turns_dropping_noise_records() {
        let session = [
            r#"{"type":"queue-operation","operation":"enqueue"}"#,
            r#"{"type":"user","message":{"role":"user","content":"how do I borrow check"}}"#,
            r#"{"type":"ai-title","aiTitle":"Borrow chat"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"use a reference"}]}}"#,
        ]
        .join("\n");

        let turns = parse_transcript(&session);

        assert_eq!(turns.len(), 2, "only the two Messages become turns");
        assert_eq!(turns[0].number, 1);
        assert_eq!(turns[0].kind, TurnKind::Prompt);
        assert_eq!(turns[0].blocks, vec![TurnBlock::Text("how do I borrow check".into())]);
        assert_eq!(turns[1].number, 2);
        assert_eq!(turns[1].kind, TurnKind::Reply);
    }

    #[test]
    fn a_tool_use_keeps_only_its_key_argument() {
        let session = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/lib.rs","limit":50}}]}}"#;

        let turns = parse_transcript(session);

        assert_eq!(
            turns[0].blocks,
            vec![TurnBlock::ToolUse { name: "Read".into(), arg: Some("src/lib.rs".into()) }]
        );
    }

    #[test]
    fn a_failed_tool_result_is_labelled_with_the_tool_that_produced_it() {
        // The tool name is joined from the assistant tool_use via tool_use_id.
        let session = [
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","is_error":true,"content":"Exit code 101"}]}}"#,
        ]
        .join("\n");

        let turns = parse_transcript(&session);

        assert_eq!(turns[1].kind, TurnKind::ToolOutput, "a pure tool_result turn is header-less");
        assert_eq!(
            turns[1].blocks,
            vec![TurnBlock::ToolResult {
                is_error: true,
                tool: Some("Bash".into()),
                text: "Exit code 101".into(),
            }]
        );
    }

    #[test]
    fn renders_speaker_labels_and_a_compact_tool_one_liner() {
        let turns = vec![
            Turn { number: 1, kind: TurnKind::Prompt, blocks: vec![TurnBlock::Text("fix the build".into())] },
            Turn {
                number: 2,
                kind: TurnKind::Reply,
                blocks: vec![
                    TurnBlock::Text("Let me look.".into()),
                    TurnBlock::ToolUse { name: "Bash".into(), arg: Some("cargo build".into()) },
                ],
            },
        ];

        let out = format_transcript(&turns, false);

        assert!(out.contains("you\n  fix the build"), "user header + prompt: {out}");
        assert!(out.contains("claude\n  Let me look."), "assistant header + reply: {out}");
        assert!(out.contains("→ Bash cargo build"), "compact tool one-liner: {out}");
        assert!(!out.contains("{"), "no raw JSON in the transcript: {out}");
    }

    #[test]
    fn a_failed_tool_result_is_flagged_loudly() {
        let turns = vec![Turn {
            number: 1,
            kind: TurnKind::ToolOutput,
            blocks: vec![TurnBlock::ToolResult {
                is_error: true,
                tool: Some("Bash".into()),
                text: "error[E0433]: failed to resolve".into(),
            }],
        }];

        let out = format_transcript(&turns, false);

        assert!(out.contains("✗ Bash FAILED"), "failure flagged loudly: {out}");
        assert!(out.contains("E0433"), "error text carried through: {out}");
        assert!(!out.starts_with("you"), "tool output has no speaker header: {out}");
    }

    #[test]
    fn thinking_is_collapsed_by_default_and_expands_with_the_flag() {
        let turns = vec![Turn {
            number: 1,
            kind: TurnKind::Reply,
            blocks: vec![TurnBlock::Thinking("step one\nstep two\nstep three".into())],
        }];

        let collapsed = format_transcript(&turns, false);
        assert!(collapsed.contains("[thinking: 3 lines hidden"), "collapsed with a count: {collapsed}");
        assert!(!collapsed.contains("step two"), "thinking text hidden by default: {collapsed}");

        let expanded = format_transcript(&turns, true);
        assert!(expanded.contains("step two"), "--thinking reveals the text: {expanded}");
    }

    #[test]
    fn a_signature_only_thinking_block_is_dropped_not_rendered_as_a_phantom_turn() {
        // Real data: a `thinking` block can carry only a signature and an empty
        // `thinking` string. It must not become a turn or a misleading count.
        let session = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"","signature":"abc"}]}}"#;

        assert!(parse_transcript(session).is_empty(), "empty thinking yields no turn");
    }

    #[test]
    fn an_empty_session_renders_a_clear_placeholder() {
        assert_eq!(format_transcript(&parse_transcript(""), false), "(no messages)\n");
    }

    /// `n` plain Prompt turns numbered 1..=n, each carrying a unique marker.
    fn numbered_turns(n: usize) -> Vec<Turn> {
        (1..=n)
            .map(|i| Turn {
                number: i,
                kind: TurnKind::Prompt,
                blocks: vec![TurnBlock::Text(format!("turn-{i}-text"))],
            })
            .collect()
    }

    #[test]
    fn window_selects_the_turns_around_the_target_and_counts_the_rest() {
        let turns = numbered_turns(10);

        let (window, above, below) = window_turns(&turns, 5, 2);

        let nums: Vec<usize> = window.iter().map(|t| t.number).collect();
        assert_eq!(nums, vec![3, 4, 5, 6, 7], "turns 5±2");
        assert_eq!(above, 2, "turns 1,2 hidden above");
        assert_eq!(below, 3, "turns 8,9,10 hidden below");
    }

    #[test]
    fn window_clamps_at_the_start() {
        let turns = numbered_turns(10);

        let (window, above, below) = window_turns(&turns, 1, 3);

        assert_eq!(window.first().map(|t| t.number), Some(1));
        assert_eq!(window.last().map(|t| t.number), Some(4));
        assert_eq!(above, 0, "nothing hidden before turn 1");
        assert_eq!(below, 6);
    }

    #[test]
    fn window_clamps_at_the_end() {
        let turns = numbered_turns(10);

        let (window, above, below) = window_turns(&turns, 10, 3);

        assert_eq!(window.first().map(|t| t.number), Some(7));
        assert_eq!(window.last().map(|t| t.number), Some(10));
        assert_eq!(above, 6);
        assert_eq!(below, 0, "nothing hidden after the last turn");
    }

    #[test]
    fn format_windowed_renders_only_the_window_with_hidden_indicators() {
        let turns = numbered_turns(10);

        let out = format_windowed(&turns, 5, 2, false);

        assert!(out.contains("turn-5-text"), "the target turn is shown: {out}");
        assert!(!out.contains("turn-2-text") && !out.contains("turn-8-text"), "outside hidden: {out}");
        assert!(out.contains("2 earlier turns hidden"), "above indicator: {out}");
        assert!(out.contains("3 later turns hidden"), "below indicator: {out}");
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
