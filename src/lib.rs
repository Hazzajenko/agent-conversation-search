//! `ccsearch` — search your local Claude Code conversation history.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use regex::RegexBuilder;

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
/// (Prompts, Replies, Titles). [`parse_line`] emits every kind tagged by
/// [`Role`]; this is the policy the search pipeline applies to decide what to
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

/// Extract **every** searchable [`Segment`] a single JSONL line contributes,
/// each tagged with its [`Role`] (Prompt, Reply, Title, Thinking, tool call or
/// tool result). This parser applies no content policy — the search pipeline
/// decides which roles to actually match via a [`ContentSet`].
///
/// Parsing is **lenient**: a line that is not valid JSON, or whose shape we do
/// not recognise, contributes no Segments instead of failing. This keeps a
/// single malformed or future-versioned Record from aborting a search.
pub fn parse_line(line: &str) -> Vec<Segment> {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(value) => segments_from_value(&value),
        Err(_) => Vec::new(),
    }
}

/// The [`Segment`]s carried by an already-parsed Record. Split out from
/// [`parse_line`] so the search pipeline can parse each line once and read both
/// its Segments and its metadata (timestamp, branch) from the same value.
fn segments_from_value(value: &serde_json::Value) -> Vec<Segment> {
    match value.get("type").and_then(|t| t.as_str()) {
        Some("user") => match value.pointer("/message/content") {
            // A plain string is a Prompt.
            Some(c) if c.is_string() => {
                vec![Segment { role: Role::User, text: c.as_str().unwrap().to_string() }]
            }
            // An array is tool_result content, not a Prompt.
            Some(c) if c.is_array() => {
                c.as_array().unwrap().iter().filter_map(user_block_segment).collect()
            }
            _ => Vec::new(),
        },
        Some("assistant") => value
            .pointer("/message/content")
            .and_then(|c| c.as_array())
            .map(|blocks| blocks.iter().filter_map(assistant_block_segment).collect())
            .unwrap_or_default(),
        Some("ai-title") => match value.get("aiTitle").and_then(|t| t.as_str()) {
            Some(text) => vec![Segment { role: Role::Title, text: text.to_string() }],
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Map one block of a user Message's `content` array to the [`Segment`] it
/// contributes. User array content is tool_result content (a Prompt is a plain
/// string, handled separately). A tool_result's own `content` may be a string
/// or an array of text blocks.
fn user_block_segment(block: &serde_json::Value) -> Option<Segment> {
    match block.get("type").and_then(|t| t.as_str()) {
        Some("tool_result") => {
            let text = match block.get("content") {
                Some(c) if c.is_string() => c.as_str().unwrap().to_string(),
                Some(c) if c.is_array() => c
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => return None,
            };
            Some(Segment { role: Role::Tool, text })
        }
        _ => None,
    }
}

/// Map one block of an assistant Message's `content` array to the [`Segment`]
/// it contributes, or `None` for blocks that carry no searchable text. Every
/// recognised block is tagged with its [`Role`]; the search pipeline decides
/// which roles to actually search.
fn assistant_block_segment(block: &serde_json::Value) -> Option<Segment> {
    match block.get("type").and_then(|t| t.as_str()) {
        Some("text") => block
            .get("text")
            .and_then(|t| t.as_str())
            .map(|text| Segment { role: Role::Assistant, text: text.to_string() }),
        Some("thinking") => block
            .get("thinking")
            .and_then(|t| t.as_str())
            .map(|text| Segment { role: Role::Thinking, text: text.to_string() }),
        Some("tool_use") => {
            // Make both the tool name and its input arguments searchable.
            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let input = block.get("input").map(|i| i.to_string()).unwrap_or_default();
            Some(Segment { role: Role::Tool, text: format!("{name} {input}").trim().to_string() })
        }
        _ => None,
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
    /// The Matches, in the order they appear in the Session.
    pub matches: Vec<Match>,
}

/// Search the given Project directories for `query`, returning one
/// [`SessionMatches`] per Session that contains at least one Match.
///
/// Matching is delegated to `matcher` over the default content set (see
/// [`parse_line`]). Sessions with no Matches are omitted. Unreadable directories
/// and files are skipped rather than failing the whole search.
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
    let mut title = None;
    let mut timestamp: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut matches = Vec::new();
    // Turn number = position among user/assistant Message Records, in file
    // order — the same numbering `parse_transcript` assigns, so the turn a
    // search hit reports lands `show --around <turn>` on the same Message.
    let mut turn = 0;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(ts) = value.get("timestamp").and_then(|t| t.as_str()) {
            // ISO 8601 sorts lexically, so the max string is the newest record.
            if timestamp.as_deref().is_none_or(|cur| ts > cur) {
                timestamp = Some(ts.to_string());
            }
        }
        if branch.is_none() {
            if let Some(b) = value.get("gitBranch").and_then(|b| b.as_str()) {
                branch = Some(b.to_string());
            }
        }
        let is_message = matches!(value.get("type").and_then(|t| t.as_str()), Some("user") | Some("assistant"));
        if is_message {
            turn += 1;
        }
        for seg in segments_from_value(&value) {
            if seg.role == Role::Title {
                title = Some(seg.text.clone());
            }
            if content.includes(seg.role) && matcher.is_match(&seg.text) {
                // A Title comes from an `ai-title` Record, not a turn.
                let turn = if seg.role == Role::Title { None } else { Some(turn) };
                matches.push(Match { turn, segment: seg });
            }
        }
    }
    if matches.is_empty() {
        return None;
    }
    let session_id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    Some(SessionMatches {
        project: project.to_string(),
        session_id,
        path: path.to_path_buf(),
        title,
        timestamp,
        branch,
        matches,
    })
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

/// Render search results as human- and Claude-readable text: each Session as a
/// `short-id · project · title · date · branch` header followed by one
/// `[turn] role: snippet` line per Match. At most `max_per_session` Matches are
/// shown per Session (`0` = unlimited), with an actionable `… +N more  ›
/// ccsearch show <id>` line when some are hidden. An empty result set renders a
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
        out.push_str(&session_header(
            &short,
            &s.project,
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
            out.push_str(&format!("  … +{hidden} more  ›  ccsearch show {short}\n"));
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

/// Keys whose value, if present, is the most informative one-liner argument for
/// a tool call. Tried in order; the first string value wins. Keeping this list
/// fixed (rather than dumping raw JSON) is what makes tool calls readable.
const TOOL_ARG_KEYS: &[&str] = &["command", "file_path", "pattern", "path", "url", "query", "prompt"];

/// The single most informative argument of a `tool_use` input object, or `None`
/// if it carries none of the known keys.
fn tool_key_arg(input: &serde_json::Value) -> Option<String> {
    TOOL_ARG_KEYS
        .iter()
        .find_map(|key| input.get(*key).and_then(|v| v.as_str()).map(str::to_string))
}

/// Parse a whole Session file into its Transcript turns (Messages only — the
/// noise Records are dropped). Done in two passes: first map every `tool_use`
/// id to its tool name, then build the turns so a failed `tool_result` can be
/// labelled with the tool that produced it.
pub fn parse_transcript(session_text: &str) -> Vec<Turn> {
    let values: Vec<serde_json::Value> = session_text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect();

    // Pass 1: tool_use_id -> tool name, for labelling tool results.
    let mut tool_names: HashMap<String, String> = HashMap::new();
    for value in &values {
        if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(blocks) = value.pointer("/message/content").and_then(|c| c.as_array()) else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            if let (Some(id), Some(name)) = (
                block.get("id").and_then(|i| i.as_str()),
                block.get("name").and_then(|n| n.as_str()),
            ) {
                tool_names.insert(id.to_string(), name.to_string());
            }
        }
    }

    // Pass 2: build turns, numbering every Message Record 1-based in Record
    // order. Contentless Records (e.g. signature-only thinking) still consume a
    // number but are not pushed — search numbers them identically, so the
    // numbers stay aligned across the find→read handoff (ADR 0002).
    let mut turns = Vec::new();
    let mut number = 0;
    for value in &values {
        let ty = value.get("type").and_then(|t| t.as_str());
        if matches!(ty, Some("user") | Some("assistant")) {
            number += 1;
        }
        let blocks = match ty {
            Some("user") => match value.pointer("/message/content") {
                Some(c) if c.is_string() => {
                    let prompt = c.as_str().unwrap();
                    if prompt.trim().is_empty() {
                        Vec::new()
                    } else {
                        vec![TurnBlock::Text(prompt.to_string())]
                    }
                }
                Some(c) if c.is_array() => {
                    c.as_array().unwrap().iter().filter_map(|b| user_turn_block(b, &tool_names)).collect()
                }
                _ => Vec::new(),
            },
            Some("assistant") => value
                .pointer("/message/content")
                .and_then(|c| c.as_array())
                .map(|bs| bs.iter().filter_map(assistant_turn_block).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if blocks.is_empty() {
            continue;
        }
        let kind = match ty {
            Some("assistant") => TurnKind::Reply,
            // A user Message that is *only* tool results is mechanical output,
            // not something the person typed.
            _ if blocks.iter().all(|b| matches!(b, TurnBlock::ToolResult { .. })) => {
                TurnKind::ToolOutput
            }
            _ => TurnKind::Prompt,
        };
        turns.push(Turn { number, kind, blocks });
    }
    turns
}

/// The value of `block[key]` as an owned string, but only if it is present and
/// not blank. Used to drop empty `text`/`thinking` blocks (e.g. signature-only
/// thinking) so they do not render as phantom turns.
fn non_empty_text(block: &serde_json::Value, key: &str) -> Option<String> {
    block
        .get(key)
        .and_then(|t| t.as_str())
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
}

/// Map one block of a user Message's `content` array to a [`TurnBlock`]. User
/// array content is tool results (and occasionally text); a `tool_result`'s own
/// `content` may be a string or an array of text blocks.
fn user_turn_block(block: &serde_json::Value, tool_names: &HashMap<String, String>) -> Option<TurnBlock> {
    match block.get("type").and_then(|t| t.as_str()) {
        Some("text") => non_empty_text(block, "text").map(TurnBlock::Text),
        Some("tool_result") => {
            let is_error = block.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
            let text = tool_result_text(block);
            let tool = block
                .get("tool_use_id")
                .and_then(|i| i.as_str())
                .and_then(|id| tool_names.get(id).cloned());
            Some(TurnBlock::ToolResult { is_error, tool, text })
        }
        _ => None,
    }
}

/// The text of a `tool_result` block: its `content`, which may be a plain
/// string or an array of text blocks (joined). Empty when neither is present.
fn tool_result_text(block: &serde_json::Value) -> String {
    match block.get("content") {
        Some(c) if c.is_string() => c.as_str().unwrap().to_string(),
        Some(c) if c.is_array() => c
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Map one block of an assistant Message's `content` array to a [`TurnBlock`],
/// or `None` for blocks that render nothing.
fn assistant_turn_block(block: &serde_json::Value) -> Option<TurnBlock> {
    match block.get("type").and_then(|t| t.as_str()) {
        Some("text") => non_empty_text(block, "text").map(TurnBlock::Text),
        // Signature-only `thinking` blocks carry no readable text — drop them
        // rather than render a phantom turn.
        Some("thinking") => non_empty_text(block, "thinking").map(TurnBlock::Thinking),
        Some("tool_use") => {
            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool").to_string();
            let arg = block.get("input").and_then(tool_key_arg);
            Some(TurnBlock::ToolUse { name, arg })
        }
        _ => None,
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
}

const fn fm(needle: &'static str, label: &'static str) -> FailureMarker {
    FailureMarker { needle, label }
}

/// Markers that classify a tool Failure, in **priority order**. Applied
/// top-to-bottom: the first whose `needle` matches any line wins, so a compiler
/// `error[` outranks a generic `Error:` regardless of where each appears. A
/// fixed, documented lookup table — not a relevance ranker (ADR 0002 / 0003).
///
/// Ordering: specific code errors first, then common tool errors (issue 14),
/// then harness noise last — a real error always out-ranks an incidental match.
/// (Real data: the naive "first line" is just `Exit code 101`; the real error is
/// several lines below it — issue 11.)
const FAILURE_MARKERS: &[FailureMarker] = &[
    // Code / command errors.
    fm("error[", "error["),
    fm("panicked at", "panicked at"),
    fm("assertion failed", "assertion failed"),
    fm("assertion `", "assertion `"),
    fm("Error:", "Error:"),
    fm("does not exist", "does not exist"),
    fm("No such file", "No such file"),
    fm("unexpected EOF", "unexpected EOF"),
    fm("command not found", "command not found"),
    fm("fatal:", "fatal:"),
    fm("Traceback", "Traceback"),
    fm("error:", "error:"),
    // Common tool errors that match no code marker (issue 14).
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

/// The Failure's **signature**: the matched marker's display `label`, or `None`
/// when nothing matches. This is the grouping key for `stats` (ADR 0003); it
/// shares [`matched_marker`] with [`salient_line`] so the list and the aggregate
/// table can never disagree about a Failure's class.
fn failure_signature(error_text: &str) -> Option<&'static str> {
    matched_marker(error_text).map(|m| m.label)
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
    let values: Vec<serde_json::Value> =
        text.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).collect();

    // Pass 1: tool_use_id -> (tool name, command), for the join.
    let mut tools: HashMap<String, (String, Option<String>)> = HashMap::new();
    for value in &values {
        if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(blocks) = value.pointer("/message/content").and_then(|c| c.as_array()) else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool").to_string();
                let command = block.get("input").and_then(tool_key_arg);
                tools.insert(id.to_string(), (name, command));
            }
        }
    }

    // Pass 2: metadata, turn numbering, and the errored tool_results.
    let mut title = None;
    let mut timestamp: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut failures = Vec::new();
    let mut turn = 0;
    for value in &values {
        if let Some(ts) = value.get("timestamp").and_then(|t| t.as_str()) {
            if timestamp.as_deref().is_none_or(|cur| ts > cur) {
                timestamp = Some(ts.to_string());
            }
        }
        if branch.is_none() {
            if let Some(b) = value.get("gitBranch").and_then(|b| b.as_str()) {
                branch = Some(b.to_string());
            }
        }
        let ty = value.get("type").and_then(|t| t.as_str());
        if ty == Some("ai-title") {
            if let Some(t) = value.get("aiTitle").and_then(|t| t.as_str()) {
                title = Some(t.to_string());
            }
        }
        if matches!(ty, Some("user") | Some("assistant")) {
            turn += 1;
        }
        if ty != Some("user") {
            continue;
        }
        let Some(blocks) = value.pointer("/message/content").and_then(|c| c.as_array()) else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
                continue;
            }
            if block.get("is_error").and_then(|e| e.as_bool()) != Some(true) {
                continue;
            }
            let error_text = tool_result_text(block);
            let (tool, command) = block
                .get("tool_use_id")
                .and_then(|i| i.as_str())
                .and_then(|id| tools.get(id))
                .map_or((None, None), |(name, command)| (Some(name.clone()), command.clone()));
            failures.push(Failure {
                turn: Some(turn),
                tool,
                command,
                exit_code: exit_code(&error_text),
                error_text,
            });
        }
    }

    // Query filter (optional under --failed): match command or error text.
    if let Some(matcher) = matcher {
        failures.retain(|f| {
            f.command.as_deref().is_some_and(|c| matcher.is_match(c)) || matcher.is_match(&f.error_text)
        });
    }

    if failures.is_empty() {
        return None;
    }
    let session_id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    Some(SessionFailures {
        project: project.to_string(),
        session_id,
        path: path.to_path_buf(),
        title,
        timestamp,
        branch,
        failures,
    })
}

/// One row of the `stats` table (ADR 0003): a count of Failures sharing the
/// same `(tool, signature)`, where `signature` is the matched [`FAILURE_MARKERS`]
/// entry (or `None` — one no-marker bucket per tool).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureGroup {
    pub tool: Option<String>,
    pub signature: Option<&'static str>,
    pub count: usize,
}

/// Fold every Failure in scope into [`FailureGroup`]s keyed by `(tool, signature)`
/// — the aggregate counterpart to [`format_failures`]'s per-Failure list. Sorted
/// by count descending, then tool then signature, so output is deterministic
/// regardless of scan order.
pub fn group_failures(results: &[SessionFailures]) -> Vec<FailureGroup> {
    let mut counts: HashMap<(Option<String>, Option<&'static str>), usize> = HashMap::new();
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
            &s.project,
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
            out.push_str(&format!("  … +{hidden} more  ›  ccsearch show {short}\n"));
        }
        out.push('\n');
    }
    out
}

/// Render the `stats` table (ADR 0003): one right-aligned `<count>  ✗ <tool>
/// <signature>` row per [`FailureGroup`], biggest first, with `(no marker)` for
/// the unmatched bucket. Reports cleanly when there are none, like
/// [`format_failures`].
pub fn format_stats(groups: &[FailureGroup]) -> String {
    if groups.is_empty() {
        return "No failures.\n".to_string();
    }
    let count_width = groups.iter().map(|g| g.count.to_string().len()).max().unwrap_or(1);
    // Cap the tool column so one long name (e.g. a verbose MCP tool) cannot
    // sparse-out every other row; longer names simply overflow past the pad.
    const TOOL_WIDTH_CAP: usize = 16;
    let tool_width = groups
        .iter()
        .map(|g| g.tool.as_deref().unwrap_or("tool").chars().count())
        .max()
        .unwrap_or(0)
        .min(TOOL_WIDTH_CAP);
    let mut out = String::new();
    for g in groups {
        let tool = g.tool.as_deref().unwrap_or("tool");
        let signature = g.signature.unwrap_or("(no marker)");
        out.push_str(&format!(
            "{:>count_width$}  ✗ {:<tool_width$}  {}\n",
            g.count, tool, signature
        ));
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
/// `days_from_civil`). Hand-rolled to keep `ccsearch` free of a date-library
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
        fs::create_dir(root.join("E--projects-rust-ccsearch")).unwrap();
        fs::create_dir(root.join("C--hacking-jplag")).unwrap();

        let dirs = resolve_scope(root, &Scope::Project { name_substring: "PROJECTS".into() });

        assert_eq!(
            dirs,
            vec![
                root.join("E--projects-games-creature-game"),
                root.join("E--projects-rust-ccsearch"),
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
        // must equal the turn `show` numbers that Message. The tricky case is a
        // contentless Record (here a signature-only thinking block) — it must
        // consume a turn number in *both* views or every later turn drifts.
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

    /// Build a [`SessionMatches`] from bare Segments, assigning each a 1-based
    /// turn for tests that only care about rendering, not turn alignment.
    fn session(project: &str, title: Option<&str>, segments: Vec<Segment>) -> SessionMatches {
        let matches = segments
            .into_iter()
            .enumerate()
            .map(|(i, segment)| Match { turn: Some(i + 1), segment })
            .collect();
        SessionMatches {
            project: project.into(),
            session_id: "11111111-2222-3333-4444-555555555555".into(),
            path: PathBuf::from("/x/11111111-2222-3333-4444-555555555555.jsonl"),
            title: title.map(Into::into),
            timestamp: None,
            branch: None,
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
            project: "p".into(),
            session_id: "abcd1234-rest".into(),
            path: PathBuf::from("/x/abcd1234-rest.jsonl"),
            title: Some("Borrow chat".into()),
            timestamp: None,
            branch: None,
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
            out.contains("ccsearch show 11111111"),
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
    fn parses_an_assistant_thinking_block_into_a_thinking_segment() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"the secret password is hunter2"}
        ]}}"#;
        assert_eq!(
            parse_line(line),
            vec![Segment { role: Role::Thinking, text: "the secret password is hunter2".into() }]
        );
    }

    #[test]
    fn parses_an_assistant_tool_use_block_into_a_tool_segment_with_name_and_input() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}]}}"#;
        let segs = parse_line(line);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].role, Role::Tool);
        assert!(segs[0].text.contains("Bash"), "tool name searchable: {:?}", segs[0].text);
        assert!(segs[0].text.contains("cargo test"), "tool input searchable: {:?}", segs[0].text);
    }

    #[test]
    fn parses_a_user_tool_result_into_a_tool_segment() {
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"pasted file contents here"}]}}"#;
        assert_eq!(
            parse_line(line),
            vec![Segment { role: Role::Tool, text: "pasted file contents here".into() }]
        );
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

    #[test]
    fn parses_assistant_text_and_thinking_blocks_tagged_by_role_in_order() {
        // parse_line emits every text-bearing block tagged with its Role; the
        // search pipeline (not the parser) decides which roles to search.
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"thinking","thinking":"internal reasoning"},
            {"type":"text","text":"You can use a reference."},
            {"type":"text","text":"Then run it."}
        ]}}"#;
        assert_eq!(
            parse_line(line),
            vec![
                Segment { role: Role::Thinking, text: "internal reasoning".into() },
                Segment { role: Role::Assistant, text: "You can use a reference.".into() },
                Segment { role: Role::Assistant, text: "Then run it.".into() },
            ]
        );
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
    fn failure_signature_is_the_highest_priority_matched_marker() {
        // `error[` outranks `error:`/`Error:` regardless of line order.
        let text = "Exit code 101\nerror: aborting\nerror[E0433]: cannot find type";
        assert_eq!(failure_signature(text), Some("error["));
    }

    #[test]
    fn failure_signature_is_none_when_no_marker_matches() {
        assert_eq!(failure_signature("just some unremarkable stdout\nwith nothing notable"), None);
    }

    #[test]
    fn failure_signature_and_salient_line_agree_on_the_marker() {
        // The list (salient_line) and the table (failure_signature) must never
        // disagree about a Failure's class — they share the marker lookup.
        let text = "Exit code 101\nrunning 1 test\nthread 'main' panicked at src/lib.rs:5:9:";
        assert_eq!(failure_signature(text), Some("panicked at"));
        assert!(salient_line(text).contains("panicked at"));
    }

    #[test]
    fn failure_signature_unwraps_tool_use_error_and_classifies_the_inner_message() {
        // The wrapper must not defeat classification (issue 14).
        let text = "<tool_use_error>String to replace not found in file.</tool_use_error>";
        assert_eq!(failure_signature(text), Some("String to replace not found"));
    }

    #[test]
    fn failure_signature_classifies_the_common_unmarked_tool_errors() {
        assert_eq!(
            failure_signature("File has not been read yet. Read it first before writing to it."),
            Some("has not been read"),
        );
        assert_eq!(
            failure_signature("File content (38631 tokens) exceeds maximum allowed tokens (25000)."),
            Some("exceeds maximum allowed tokens"),
        );
        assert_eq!(
            failure_signature("foo : The term 'foo' is not recognized as the name of a cmdlet"),
            Some("is not recognized"),
        );
        assert_eq!(
            failure_signature("<tool_use_error>Blocked: sleep 90 followed by: cat x</tool_use_error>"),
            Some("Blocked"),
        );
    }

    #[test]
    fn failure_signature_buckets_harness_noise_under_clean_labels() {
        // Structurally is_error, but the tool did not error — bucketed, not
        // filtered, under a short label distinct from the matched substring.
        let rejected = "The user doesn't want to proceed with this tool use. The tool use was rejected";
        assert_eq!(failure_signature(rejected), Some("rejected"));
        let unavailable = "claude-opus-4-8 is temporarily unavailable, so auto mode cannot determine safety";
        assert_eq!(failure_signature(unavailable), Some("unavailable"));
        // A parallel-batch sibling errored, so this call was cancelled unrun.
        let cancelled = "Cancelled: parallel tool call Bash(cd \"D:\\x\" && git status) errored";
        assert_eq!(failure_signature(cancelled), Some("cancelled"));
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
            failures,
        }
    }

    #[test]
    fn group_failures_counts_by_tool_and_marker_signature() {
        let s = session_of(vec![
            failure("PowerShell", "Exit code 101\nerror[E0433]: cannot find type `A`"),
            failure("PowerShell", "Exit code 101\nerror[E0609]: no field `b`"),
            failure("Bash", "Exit code 2\nunexpected EOF while looking for matching `\"'"),
        ]);

        let groups = group_failures(std::slice::from_ref(&s));

        // Two PowerShell `error[` failures collapse into one group of 2; the
        // differing exit-less specifics (E0433 vs E0609) do not split them.
        let ps = groups
            .iter()
            .find(|g| g.tool.as_deref() == Some("PowerShell") && g.signature == Some("error["))
            .expect("a PowerShell error[ group");
        assert_eq!(ps.count, 2);
        let bash = groups
            .iter()
            .find(|g| g.tool.as_deref() == Some("Bash") && g.signature == Some("unexpected EOF"))
            .expect("a Bash unexpected-EOF group");
        assert_eq!(bash.count, 1);
    }

    #[test]
    fn group_failures_buckets_unmatched_failures_under_a_none_signature() {
        let s = session_of(vec![
            failure("Bash", "some unremarkable output\nnothing notable here"),
            failure("Glob", "another line the markers do not recognise"),
        ]);

        let groups = group_failures(&[s]);

        assert!(groups.iter().all(|g| g.signature.is_none()), "no marker matched either");
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
        assert!(groups.iter().any(|g| g.signature == Some("rejected")), "rejection bucketed");
        assert!(groups.iter().any(|g| g.signature == Some("unavailable")), "unavailability bucketed");
    }

    #[test]
    fn group_failures_sorts_by_count_descending() {
        let s = session_of(vec![
            failure("Bash", "fatal: not a git repository"),
            failure("PowerShell", "error[E0433]: x"),
            failure("PowerShell", "error[E0609]: y"),
            failure("PowerShell", "error[E0425]: z"),
        ]);

        let groups = group_failures(&[s]);

        assert_eq!(groups.first().unwrap().count, 3, "the biggest group leads");
        assert_eq!(groups.first().unwrap().signature, Some("error["));
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
            FailureGroup { tool: Some("PowerShell".into()), signature: Some("error["), count: 12 },
            FailureGroup { tool: Some("Bash".into()), signature: Some("unexpected EOF"), count: 4 },
        ];

        let out = format_stats(&groups);

        assert!(out.contains("12"), "the count: {out}");
        assert!(out.contains("✗ PowerShell"), "tool with the failed glyph: {out}");
        assert!(out.contains("error["), "the marker signature: {out}");
        assert!(out.contains("unexpected EOF"));
        // Counts are right-aligned to a common width, so 4 trails 12.
        let twelve = out.find("12").unwrap();
        let four = out.find(" 4").unwrap();
        assert!(twelve < four, "biggest count first: {out}");
    }

    #[test]
    fn format_stats_labels_the_none_signature_bucket() {
        let groups = vec![FailureGroup { tool: Some("Read".into()), signature: None, count: 5 }];
        assert!(format_stats(&groups).contains("(no marker)"), "names the unmatched bucket");
    }

    #[test]
    fn format_stats_caps_the_tool_column_so_a_long_name_does_not_sparse_out_rows() {
        let groups = vec![
            FailureGroup { tool: Some("PowerShell".into()), signature: Some("error["), count: 2 },
            FailureGroup {
                tool: Some("mcp__ccd_session_mgmt__search_session_transcripts".into()),
                signature: None,
                count: 1,
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
