//! Harness adapters and Store aggregation (ADR 0010).

use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::session::Session;
use crate::{ProjectKey, Scope, SessionIdentity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Harness {
    Claude,
    Codex,
}

/// A Session id qualified by its Harness. Session ids can coincide across
/// Stores, so Store-wide filters must use both fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SessionKey {
    pub harness: Harness,
    pub session_id: String,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

pub(crate) trait HarnessAdapter: Send + Sync {
    fn harness(&self) -> Harness;
    fn recognizes(&self, text: &str) -> bool;
    fn parse_text(&self, text: &str) -> Option<Session>;
    fn enumerate(&self, include_subagents: bool) -> Vec<SessionIdentity>;
    fn matching(&self, prefix: &str, include_subagents: bool) -> Vec<SessionIdentity>;

    fn enumerate_including_subagents(&self) -> Vec<SessionIdentity> {
        self.enumerate(true)
    }

    fn matching_including_subagents(&self, prefix: &str) -> Vec<SessionIdentity> {
        self.matching(prefix, true)
    }

    fn parse(&self, path: &Path) -> Option<Session> {
        let text = std::fs::read_to_string(path).ok()?;
        self.recognizes(&text)
            .then(|| self.parse_text(&text))
            .flatten()
    }
}

pub(crate) struct ClaudeAdapter {
    projects_root: PathBuf,
}

impl ClaudeAdapter {
    pub(crate) fn discover(claude_dir: &Path) -> Self {
        Self {
            projects_root: claude_dir.join("projects"),
        }
    }

    fn session_paths(&self, include_subagents: bool) -> Vec<(String, String, PathBuf)> {
        let Ok(projects) = std::fs::read_dir(&self.projects_root) else {
            return Vec::new();
        };
        let mut sessions = Vec::new();
        for project in projects
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|ty| ty.is_dir()))
        {
            let project_name = project.file_name().to_string_lossy().into_owned();
            let Ok(files) = std::fs::read_dir(project.path()) else {
                continue;
            };
            for entry in files.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                    let session_id = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    if !session_id.is_empty() {
                        sessions.push((project_name.clone(), session_id, path));
                    }
                    continue;
                }
                if include_subagents && entry.file_type().is_ok_and(|ty| ty.is_dir()) {
                    let mut nested = Vec::new();
                    visit_jsonl(&path.join("subagents"), &mut nested);
                    for nested_path in nested {
                        let session_id = nested_path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        if !session_id.is_empty() {
                            sessions.push((project_name.clone(), session_id, nested_path));
                        }
                    }
                }
            }
        }
        sessions
    }

    fn session_info(
        &self,
        project: String,
        session_id: String,
        path: PathBuf,
    ) -> Option<SessionIdentity> {
        let parsed = self.parse(&path)?;
        let parent_id = claude_parent_id(&path);
        Some(SessionIdentity {
            harness: Harness::Claude,
            subagent: None,
            project: Some(ProjectKey::from_encoded(project)),
            session_id,
            path,
            title: parsed.meta.title,
            timestamp: parsed.meta.timestamp,
            branch: parsed.meta.branch,
            cwd: parsed.meta.cwd,
            parent_id,
        })
    }

    fn sessions(&self, prefix: Option<&str>, include_subagents: bool) -> Vec<SessionIdentity> {
        self.session_paths(include_subagents)
            .into_iter()
            .filter(|(_, session_id, _)| prefix.is_none_or(|prefix| session_id.starts_with(prefix)))
            .filter_map(|(project, session_id, path)| self.session_info(project, session_id, path))
            .collect()
    }
}

impl HarnessAdapter for ClaudeAdapter {
    fn harness(&self) -> Harness {
        Harness::Claude
    }

    fn recognizes(&self, _text: &str) -> bool {
        true
    }

    fn parse_text(&self, text: &str) -> Option<Session> {
        Some(crate::session::read(text))
    }

    fn enumerate(&self, _include_subagents: bool) -> Vec<SessionIdentity> {
        self.sessions(None, false)
    }

    fn matching(&self, prefix: &str, _include_subagents: bool) -> Vec<SessionIdentity> {
        self.sessions(Some(prefix), false)
    }

    fn enumerate_including_subagents(&self) -> Vec<SessionIdentity> {
        self.sessions(None, true)
    }

    fn matching_including_subagents(&self, prefix: &str) -> Vec<SessionIdentity> {
        self.sessions(Some(prefix), true)
    }
}

pub(crate) struct CodexAdapter {
    codex_dir: PathBuf,
    titles: std::collections::HashMap<String, String>,
}

impl CodexAdapter {
    pub(crate) fn discover(codex_dir: &Path) -> Self {
        Self {
            codex_dir: codex_dir.to_path_buf(),
            titles: codex_titles(&codex_dir.join("session_index.jsonl")),
        }
    }

    fn for_session_path(path: &Path) -> Self {
        codex_store_root(path).map_or_else(
            || Self {
                codex_dir: PathBuf::new(),
                titles: Default::default(),
            },
            |root| Self::discover(&root),
        )
    }

    fn rollout_paths(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        visit_jsonl(&self.codex_dir.join("sessions"), &mut files);
        files.sort();
        files
    }

    fn sessions(&self, prefix: Option<&str>, include_subagents: bool) -> Vec<SessionIdentity> {
        self.rollout_paths()
            .into_iter()
            .filter_map(|path| {
                if let Some(prefix) = prefix {
                    let first = read_first_line(&path)?;
                    let meta = codex_meta(&first)?;
                    let session_id = meta
                        .pointer("/payload/id")
                        .or_else(|| meta.pointer("/payload/session_id"))?
                        .as_str()?;
                    if !session_id.starts_with(prefix) {
                        return None;
                    }
                }
                let text = std::fs::read_to_string(&path).ok()?;
                codex_session_info(path, &text, &self.titles, include_subagents)
            })
            .collect()
    }
}

impl HarnessAdapter for CodexAdapter {
    fn harness(&self) -> Harness {
        Harness::Codex
    }

    fn recognizes(&self, text: &str) -> bool {
        codex_meta(text).is_some()
    }

    fn parse_text(&self, text: &str) -> Option<Session> {
        let meta = codex_meta(text)?;
        let session_id = meta
            .pointer("/payload/id")
            .or_else(|| meta.pointer("/payload/session_id"))?
            .as_str()?;
        let title = self.titles.get(session_id).cloned();
        Some(codex_read_with_title(text, title))
    }

    fn enumerate(&self, include_subagents: bool) -> Vec<SessionIdentity> {
        self.sessions(None, include_subagents)
    }

    fn matching(&self, prefix: &str, include_subagents: bool) -> Vec<SessionIdentity> {
        self.sessions(Some(prefix), include_subagents)
    }
}

fn codex_meta(text: &str) -> Option<Value> {
    let meta: Value = serde_json::from_str(text.lines().next()?).ok()?;
    (meta.get("type").and_then(Value::as_str) == Some("session_meta")).then_some(meta)
}

fn read_first_line(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    std::io::BufReader::new(file).lines().next()?.ok()
}

fn codex_session_info(
    path: PathBuf,
    text: &str,
    titles: &std::collections::HashMap<String, String>,
    include_subagents: bool,
) -> Option<SessionIdentity> {
    let meta = codex_meta(text)?;
    let thread_source = meta
        .pointer("/payload/thread_source")
        .and_then(Value::as_str);
    if thread_source == Some("subagent") && !include_subagents {
        return None;
    }
    let session_id = meta
        .pointer("/payload/id")
        .or_else(|| meta.pointer("/payload/session_id"))?
        .as_str()?
        .to_string();
    let title = titles.get(&session_id).cloned();
    let subagent = (thread_source == Some("subagent")).then(|| {
        meta.pointer("/payload/agent_nickname")
            .or_else(|| meta.pointer("/payload/source/subagent/thread_spawn/agent_nickname"))
            .and_then(Value::as_str)
            .unwrap_or("worker")
            .to_string()
    });
    let cwd = meta
        .pointer("/payload/cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
    let parsed = codex_read_with_title(text, title.clone());
    Some(SessionIdentity {
        harness: Harness::Codex,
        subagent,
        project: cwd.as_deref().map(ProjectKey::from_cwd),
        session_id,
        path,
        title,
        timestamp: parsed.meta.timestamp,
        branch: parsed.meta.branch,
        cwd,
        parent_id: codex_parent_id(&meta),
    })
}

fn codex_titles(path: &Path) -> std::collections::HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return std::collections::HashMap::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| {
            Some((
                value.get("id")?.as_str()?.to_string(),
                value.get("thread_name")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn claude_parent_id(path: &Path) -> Option<String> {
    let subagents = path.parent()?;
    if subagents.file_name()?.to_str()? != "subagents" {
        return None;
    }
    Some(
        subagents
            .parent()?
            .file_name()?
            .to_string_lossy()
            .into_owned(),
    )
}

fn codex_parent_id(meta: &Value) -> Option<String> {
    meta.pointer("/payload/parent_thread_id")
        .or_else(|| meta.pointer("/payload/source/subagent/thread_spawn/parent_thread_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn visit_jsonl(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|ty| ty.is_dir()) {
            visit_jsonl(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

fn codex_read_with_title(text: &str, title: Option<String>) -> Session {
    let mut meta = crate::session::SessionMeta::default();
    let mut records = crate::session::RecordBuilder::default();
    if let Some(title) = title {
        meta.title = Some(title);
    }
    // Model for the usage breakdown, from the session meta when present:
    // a direct `payload.model` first, then the newer
    // `payload.base_instructions.provenance.model` (e.g. "gpt-5.6-sol").
    let mut codex_model: Option<String> = None;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(timestamp) = value.get("timestamp").and_then(Value::as_str) {
            if meta
                .timestamp
                .as_deref()
                .is_none_or(|current| timestamp > current)
            {
                meta.timestamp = Some(timestamp.to_string());
            }
        }
        if meta.cwd.is_none() {
            meta.cwd = value
                .pointer("/payload/cwd")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if meta.branch.is_none() {
            meta.branch = value
                .pointer("/payload/git/branch")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if codex_model.is_none()
            && value.get("type").and_then(Value::as_str) == Some("session_meta")
        {
            codex_model = codex_model_from_meta(&value);
        }
        // Codex per-call Usage: each `event_msg` with payload type
        // `token_count` is one call (issue 31). Newer `token_usage_record`
        // lines are a different shape and out of scope here.
        if value.get("type").and_then(Value::as_str) == Some("event_msg")
            && value.pointer("/payload/type").and_then(Value::as_str) == Some("token_count")
        {
            let timestamp = value
                .get("timestamp")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(info) = value.pointer("/payload/info") {
                if let Some(last) = info.get("last_token_usage") {
                    let usage = codex_usage_from_token_count(last, codex_model.clone());
                    // Token-count events never consume a Transcript turn;
                    // the breakdown numbers them by file order instead.
                    records.metadata(
                        Some(crate::session::RecordKind::CodexTokenCount),
                        timestamp,
                        usage,
                    );
                }
                // The final Record's cumulative totals feed the mismatch check;
                // the last one in file order wins.
                if let Some(total) = info.get("total_token_usage") {
                    // Only remember a final when it parses; a malformed final
                    // simply disables the check (no false warning).
                    if let Some(final_usage) = codex_final_from_token_count(total) {
                        meta.codex_final_total = Some(final_usage);
                    }
                }
            }
            continue;
        }
        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let kind = codex_record_kind(payload);
        // Per-Record timestamp for the usage breakdown; Codex Messages carry
        // no Usage themselves (their token_counts arrive as separate
        // `event_msg` Records above), so None here.
        let timestamp = value
            .get("timestamp")
            .and_then(Value::as_str)
            .map(str::to_string);
        if payload.get("type").and_then(Value::as_str) == Some("message") {
            records.message(kind, timestamp, None);
        } else {
            records.attached(kind, timestamp, None);
        }
    }
    Session {
        meta,
        records: records.finish(),
    }
}

/// The model name for Codex Usage rows, from the session meta when present:
/// a direct `payload.model` first, then
/// `payload.base_instructions.provenance.model`. `None` renders a blank model
/// cell rather than guessing.
fn codex_model_from_meta(meta: &Value) -> Option<String> {
    if let Some(model) = meta.pointer("/payload/model").and_then(Value::as_str) {
        return Some(model.to_string());
    }
    meta.pointer("/payload/base_instructions/provenance/model")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Build per-call [`Usage`](crate::session::Usage) from a Codex
/// `last_token_usage` object: `input_tokens`, `cache_write_input_tokens`
/// (defaulting to 0 when the file predates it), `cached_input_tokens`, and
/// `output_tokens`. `input` is normalized to non-cached input
/// (`input - cached - cache_write`, saturating) so Codex and Claude totals
/// share one formula (input + cache-write + cache-read + output) and match
/// Codex `total_tokens` (`input + output`, cached subsets). Always returns
/// `Some`, with `None` counts for missing fields rendering blank cells like a
/// Claude call without `message.usage`.
fn codex_usage_from_token_count(
    last: &Value,
    model: Option<String>,
) -> Option<crate::session::Usage> {
    let input_raw = last.get("input_tokens").and_then(Value::as_u64);
    let cache_write = last
        .get("cache_write_input_tokens")
        .and_then(Value::as_u64)
        .or(Some(0));
    let cached = last.get("cached_input_tokens").and_then(Value::as_u64);
    let output = last.get("output_tokens").and_then(Value::as_u64);
    let input = match (input_raw, cached, cache_write) {
        (Some(raw), Some(cached), Some(write)) => {
            Some(raw.saturating_sub(cached).saturating_sub(write))
        }
        (Some(raw), None, _) => Some(raw),
        _ => None,
    };
    Some(crate::session::Usage {
        input,
        cache_create: cache_write,
        cache_read: cached,
        output,
        model,
        call_id: None,
    })
}

/// Normalize a Codex `total_token_usage` object the same way as
/// [`codex_usage_from_token_count`], for the mismatch check. Model and call
/// id are unset: only the four counts participate.
fn codex_final_from_token_count(total: &Value) -> Option<crate::session::Usage> {
    let input_raw = total.get("input_tokens").and_then(Value::as_u64);
    let cache_write = total
        .get("cache_write_input_tokens")
        .and_then(Value::as_u64)
        .or(Some(0));
    let cached = total.get("cached_input_tokens").and_then(Value::as_u64);
    let output = total.get("output_tokens").and_then(Value::as_u64);
    let input = match (input_raw, cached, cache_write) {
        (Some(raw), Some(cached), Some(write)) => {
            Some(raw.saturating_sub(cached).saturating_sub(write))
        }
        (Some(raw), None, _) => Some(raw),
        _ => None,
    };
    Some(crate::session::Usage {
        input,
        cache_create: cache_write,
        cache_read: cached,
        output,
        model: None,
        call_id: None,
    })
}

fn codex_record_kind(payload: &Value) -> Option<crate::session::RecordKind> {
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => {
            let texts = payload
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("text").and_then(Value::as_str));
            match payload.get("role").and_then(Value::as_str) {
                Some("user") => Some(crate::session::RecordKind::Prompt(
                    texts.collect::<Vec<_>>().join("\n"),
                )),
                Some("assistant") => Some(crate::session::RecordKind::Assistant(
                    texts
                        .map(|text| crate::session::AssistantBlock::Text(text.to_string()))
                        .collect(),
                )),
                _ => None,
            }
        }
        Some("reasoning") => {
            let text = [payload.get("summary"), payload.get("content")]
                .into_iter()
                .flatten()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            Some(crate::session::RecordKind::Assistant(vec![
                crate::session::AssistantBlock::Thinking(text),
            ]))
        }
        Some("custom_tool_call") | Some("function_call") => {
            let id = payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let input = payload
                .get("input")
                .or_else(|| payload.get("arguments"))
                .map(|value| match value {
                    Value::String(raw) => {
                        serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
                    }
                    other => other.clone(),
                })
                .unwrap_or(Value::Null);
            Some(crate::session::RecordKind::Assistant(vec![
                crate::session::AssistantBlock::ToolUse { id, name, input },
            ]))
        }
        Some("custom_tool_call_output") | Some("function_call_output") => {
            let tool_use_id = payload
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            let output = codex_tool_output(payload.get("output"));
            Some(crate::session::RecordKind::UserBlocks(vec![
                crate::session::UserBlock::ToolResult {
                    is_error: output.is_error,
                    exit_code: output.exit_code,
                    tool_use_id,
                    text: output.text,
                },
            ]))
        }
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CodexToolOutput {
    text: String,
    is_error: bool,
    exit_code: Option<i64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct FailureSignal {
    is_error: bool,
    exit_code: Option<i64>,
}

fn codex_tool_output(output: Option<&Value>) -> CodexToolOutput {
    let raw = match output {
        Some(Value::String(text)) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|value| {
                value
                    .get("output")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| text.clone()),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Object(object)) => object
            .get("output")
            .or_else(|| object.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| serde_json::to_string(object).unwrap_or_default()),
        _ => String::new(),
    };
    let structural = output.map(codex_structured_failure).unwrap_or_default();
    let exit_code = structural.exit_code.or_else(|| codex_text_exit_code(&raw));
    let is_error = structural.is_error
        || exit_code.is_some_and(|code| code != 0)
        || raw.contains("<tool_use_error>");
    CodexToolOutput {
        text: raw,
        is_error,
        exit_code,
    }
}

fn codex_structured_failure(value: &Value) -> FailureSignal {
    match value {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .map(|parsed| codex_structured_failure(&parsed))
            .unwrap_or_default(),
        Value::Array(items) => items.iter().fold(FailureSignal::default(), |signal, item| {
            let item = codex_structured_failure(item);
            FailureSignal {
                is_error: signal.is_error || item.is_error,
                exit_code: signal.exit_code.or(item.exit_code),
            }
        }),
        Value::Object(object) => {
            let exit_code = object.get("exit_code").and_then(Value::as_i64);
            let is_error = object
                .get("isError")
                .or_else(|| object.get("is_error"))
                .and_then(Value::as_bool)
                == Some(true)
                || exit_code.is_some_and(|code| code != 0);
            let nested = object
                .get("structuredContent")
                .or_else(|| object.get("content"))
                .map(codex_structured_failure)
                .unwrap_or_default();
            FailureSignal {
                is_error: is_error || nested.is_error,
                exit_code: exit_code.or(nested.exit_code),
            }
        }
        _ => FailureSignal::default(),
    }
}

fn codex_text_exit_code(text: &str) -> Option<i64> {
    let lower = text.to_ascii_lowercase();
    ["process exited with code ", "exit code ", "exit code: "]
        .iter()
        .find_map(|marker| {
            let start = lower.find(marker)? + marker.len();
            let digits: String = lower[start..]
                .chars()
                .skip_while(|character| character.is_ascii_whitespace())
                .take_while(|character| character.is_ascii_digit() || *character == '-')
                .collect();
            digits.parse().ok()
        })
}

pub(crate) fn parse_session_path(path: &Path) -> Option<(Harness, Session)> {
    let text = std::fs::read_to_string(path).ok()?;
    let adapters: [Box<dyn HarnessAdapter>; 2] = [
        Box::new(CodexAdapter::for_session_path(path)),
        Box::new(ClaudeAdapter {
            projects_root: PathBuf::new(),
        }),
    ];
    adapters.into_iter().find_map(|adapter| {
        adapter
            .recognizes(&text)
            .then(|| {
                adapter
                    .parse_text(&text)
                    .map(|session| (adapter.harness(), session))
            })
            .flatten()
    })
}

fn codex_store_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some("sessions"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

#[derive(Clone)]
pub struct SessionHandle {
    pub info: SessionIdentity,
    adapter_index: usize,
}

pub struct Stores {
    adapters: Vec<Box<dyn HarnessAdapter>>,
    include_subagents: bool,
    /// Sessions hidden from scope enumeration: the Current Session Family
    /// under multi-Session analysis (ADR 0011). Explicit lookup ignores it.
    excluded_sessions: HashSet<SessionKey>,
}

impl Stores {
    pub fn with_claude(claude_dir: &Path) -> Self {
        Self::from_adapters(vec![Box::new(ClaudeAdapter::discover(claude_dir))])
    }

    pub fn with_codex(codex_dir: &Path) -> Self {
        Self::from_adapters(vec![Box::new(CodexAdapter::discover(codex_dir))])
    }

    pub fn with_claude_and_codex(claude_dir: &Path, codex_dir: &Path) -> Self {
        Self::from_adapters(vec![
            Box::new(ClaudeAdapter::discover(claude_dir)),
            Box::new(CodexAdapter::discover(codex_dir)),
        ])
    }

    fn from_adapters(adapters: Vec<Box<dyn HarnessAdapter>>) -> Self {
        Self {
            adapters,
            include_subagents: false,
            excluded_sessions: HashSet::new(),
        }
    }

    pub fn including_subagents(mut self, include: bool) -> Self {
        self.include_subagents = include;
        self
    }

    pub(crate) fn includes_subagents(&self) -> bool {
        self.include_subagents
    }

    /// Hide the Current Session Family from scope enumeration, so multi-Session
    /// analysis cannot return the conversation that asked for it (ADR 0011).
    /// Outside a supported Harness this hides nothing. Explicit Session lookup
    /// (`matching_sessions`, `lookup_session`) stays unfiltered: naming a
    /// Session is intent to include it.
    pub fn excluding_current_family(mut self) -> Self {
        self.excluded_sessions = crate::current::current_family_keys(&self);
        self
    }

    pub(crate) fn sessions(&self, scope: &Scope) -> Vec<SessionHandle> {
        let mut sessions = Vec::new();
        for (adapter_index, adapter) in self.adapters.iter().enumerate() {
            sessions.extend(
                adapter
                    .enumerate(self.include_subagents)
                    .into_iter()
                    .filter(|info| {
                        !self.excluded_sessions.contains(&SessionKey {
                            harness: info.harness,
                            session_id: info.session_id.clone(),
                        })
                    })
                    .filter(|info| in_scope(info, scope))
                    .map(|info| SessionHandle {
                        info,
                        adapter_index,
                    }),
            );
        }
        sessions.sort_by(|a, b| {
            b.info
                .timestamp
                .cmp(&a.info.timestamp)
                .then_with(|| a.info.path.cmp(&b.info.path))
        });
        sessions
    }

    pub(crate) fn matching_sessions(&self, prefix: &str) -> Vec<SessionHandle> {
        let mut sessions = Vec::new();
        for (adapter_index, adapter) in self.adapters.iter().enumerate() {
            sessions.extend(
                adapter
                    .matching(prefix, self.include_subagents)
                    .into_iter()
                    .map(|info| SessionHandle {
                        info,
                        adapter_index,
                    }),
            );
        }
        sessions
    }

    pub(crate) fn parse(&self, session: &SessionHandle) -> Option<Session> {
        self.adapters
            .get(session.adapter_index)?
            .parse(&session.info.path)
    }

    pub(crate) fn configured_harnesses(&self) -> Vec<Harness> {
        self.adapters
            .iter()
            .map(|adapter| adapter.harness())
            .collect()
    }

    /// Look up a Session by exact id, including subagent threads that ordinary
    /// listing hides. Current-context resolution uses this so a worker can
    /// name its calling thread without `--include-subagents`.
    pub(crate) fn lookup_session(&self, harness: Harness, id: &str) -> Option<SessionHandle> {
        let mut matches = Vec::new();
        for (adapter_index, adapter) in self
            .adapters
            .iter()
            .enumerate()
            .filter(|(_, adapter)| adapter.harness() == harness)
        {
            matches.extend(
                adapter
                    .matching_including_subagents(id)
                    .into_iter()
                    .filter(|info| info.session_id == id)
                    .map(|info| SessionHandle {
                        info,
                        adapter_index,
                    }),
            );
        }
        matches.into_iter().next()
    }

    pub(crate) fn sessions_including_subagents(&self, harness: Harness) -> Vec<SessionHandle> {
        let mut sessions = Vec::new();
        for (adapter_index, adapter) in self
            .adapters
            .iter()
            .enumerate()
            .filter(|(_, adapter)| adapter.harness() == harness)
        {
            sessions.extend(
                adapter
                    .enumerate_including_subagents()
                    .into_iter()
                    .map(|info| SessionHandle {
                        info,
                        adapter_index,
                    }),
            );
        }
        sessions
    }

    /// All Sessions in `scope` including subagent threads, with Current Family
    /// exclusion applied. Used by `usage` ranking to fold workers into parents
    /// even though ordinary enumeration hides them (Claude workers are never
    /// listed, Codex workers only with `--include-subagents`). Sorted like
    /// [`Stores::sessions`].
    pub(crate) fn sessions_including_subagents_in_scope(
        &self,
        scope: &Scope,
    ) -> Vec<SessionHandle> {
        let mut sessions = Vec::new();
        for (adapter_index, adapter) in self.adapters.iter().enumerate() {
            sessions.extend(
                adapter
                    .enumerate_including_subagents()
                    .into_iter()
                    .filter(|info| {
                        !self.excluded_sessions.contains(&SessionKey {
                            harness: info.harness,
                            session_id: info.session_id.clone(),
                        })
                    })
                    .filter(|info| in_scope(info, scope))
                    .map(|info| SessionHandle {
                        info,
                        adapter_index,
                    }),
            );
        }
        sessions.sort_by(|a, b| {
            b.info
                .timestamp
                .cmp(&a.info.timestamp)
                .then_with(|| a.info.path.cmp(&b.info.path))
        });
        sessions
    }

    /// The subtree rooted at `root_id`: the Session itself plus every
    /// descendant subagent thread (transitive) in the same Harness. Uses the
    /// unfiltered including-subagents enumeration so an explicitly selected
    /// Session's breakdown always sees its workers, regardless of scope,
    /// `--since`, or Current Family exclusion (naming a Session is intent to
    /// include it, like `--session`). Returns empty when the root is absent.
    pub(crate) fn subtree_handles(&self, harness: Harness, root_id: &str) -> Vec<SessionHandle> {
        let all = self.sessions_including_subagents(harness);
        // Parent links within this Harness.
        let parents: std::collections::HashMap<String, Option<String>> = all
            .iter()
            .map(|h| (h.info.session_id.clone(), h.info.parent_id.clone()))
            .collect();
        if !parents.contains_key(root_id) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for h in all {
            let id = h.info.session_id.clone();
            if id == root_id || walks_to_parent(&id, root_id, &parents) {
                out.push(h);
            }
        }
        // Deterministic: root first, then workers by session id.
        out.sort_by(|a, b| {
            if a.info.session_id == root_id {
                std::cmp::Ordering::Less
            } else if b.info.session_id == root_id {
                std::cmp::Ordering::Greater
            } else {
                a.info.session_id.cmp(&b.info.session_id)
            }
        });
        out
    }

    #[cfg(test)]
    pub(crate) fn with_claude_projects_root(projects_root: &Path) -> Self {
        Self::from_adapters(vec![Box::new(ClaudeAdapter {
            projects_root: projects_root.to_path_buf(),
        })])
    }
}

fn in_scope(info: &SessionIdentity, scope: &Scope) -> bool {
    match scope {
        Scope::All => true,
        Scope::Current { cwd } => info
            .project
            .as_ref()
            .is_some_and(|project| project.matches_cwd(cwd)),
        Scope::Project { name_substring } => info
            .project
            .as_ref()
            .is_some_and(|project| project.contains_ignore_case(name_substring)),
    }
}

/// Whether `id` walks to `root` through parent links (transitive, cycle-safe).
/// `parents` maps session id to its immediate parent id, if any.
fn walks_to_parent(
    id: &str,
    root: &str,
    parents: &std::collections::HashMap<String, Option<String>>,
) -> bool {
    let mut seen = HashSet::new();
    let mut current = id;
    while let Some(parent) = parents.get(current).and_then(|p| p.as_deref()) {
        if !seen.insert(current.to_string()) {
            return false;
        }
        if parent == root {
            return true;
        }
        current = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_adapter_enumerates_and_parses_sessions() {
        let store = tempfile::tempdir().unwrap();
        let project = store.path().join("projects").join("E--projects-demo");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("11111111-1111-1111-1111-111111111111.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"role":"user","content":"adapter seam"},"cwd":"E:\\projects\\demo"}"#,
        )
        .unwrap();
        let adapter = ClaudeAdapter::discover(store.path());

        let sessions = adapter.enumerate(false);
        let parsed = adapter.parse(&path).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].project.as_ref().map(ProjectKey::as_str),
            Some("E--projects-demo")
        );
        assert_eq!(parsed.records.len(), 1);
    }

    #[test]
    fn codex_adapter_enumerates_rollouts_and_parses_messages() {
        let store = tempfile::tempdir().unwrap();
        let dir = store.path().join("sessions/2026/08/28");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-example.jsonl");
        std::fs::write(
            &path,
            [
                r#"{"timestamp":"2026-08-28T10:00:00Z","type":"session_meta","payload":{"id":"c0de0001-0000-0000-0000-000000000000","cwd":"E:\\projects\\demo","thread_source":"user"}}"#,
                r#"{"timestamp":"2026-08-28T10:01:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"prompt"}]}}"#,
                r#"{"timestamp":"2026-08-28T10:02:00Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"reply"}]}}"#,
                r#"{"timestamp":"2026-08-28T10:03:00Z","type":"response_item","payload":{"type":"custom_tool_call","call_id":"bad","name":"exec","input":"{\"cmd\":\"cargo test\"}"}}"#,
                r#"{"timestamp":"2026-08-28T10:04:00Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"bad","output":"{\"exit_code\":101,\"output\":\"compile failed\"}"}}"#,
                r#"{"timestamp":"2026-08-28T10:05:00Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        std::fs::write(
            store.path().join("session_index.jsonl"),
            r#"{"id":"c0de0001-0000-0000-0000-000000000000","thread_name":"Indexed title"}"#,
        )
        .unwrap();
        let adapter = CodexAdapter::discover(store.path());

        let sessions = adapter.enumerate(false);
        let parsed = adapter.parse(&path).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].session_id,
            "c0de0001-0000-0000-0000-000000000000"
        );
        assert_eq!(
            sessions[0].project.as_ref().map(ProjectKey::as_str),
            Some("E--projects-demo")
        );
        assert_eq!(sessions[0].title.as_deref(), Some("Indexed title"));
        assert_eq!(parsed.records.len(), 5);
        assert_eq!(parsed.meta.title.as_deref(), Some("Indexed title"));
        assert_eq!(
            parsed.records[0].kind,
            crate::session::RecordKind::Prompt("prompt".into())
        );
        assert!(matches!(
            &parsed.records[1].kind,
            crate::session::RecordKind::Assistant(blocks)
                if blocks == &vec![crate::session::AssistantBlock::Text("reply".into())]
        ));
        assert!(matches!(
            &parsed.records[3].kind,
            crate::session::RecordKind::UserBlocks(blocks)
                if matches!(
                    &blocks[0],
                    crate::session::UserBlock::ToolResult {
                        is_error: true,
                        exit_code: Some(101),
                        text,
                        ..
                    } if text == "compile failed"
                )
        ));
        let turns: Vec<_> = parsed.records.iter().map(|record| record.turn).collect();
        assert_eq!(turns, vec![Some(1), Some(2), Some(3), Some(3), Some(3)]);
    }

    #[test]
    fn codex_tool_outputs_detect_structured_failures() {
        assert_eq!(
            codex_tool_output(Some(&serde_json::json!({"exit_code": 2, "output": "bad"}))),
            CodexToolOutput {
                text: "bad".into(),
                is_error: true,
                exit_code: Some(2)
            }
        );
        assert_eq!(
            codex_tool_output(Some(&serde_json::json!([
                {"text": "nested failure", "isError": true}
            ]))),
            CodexToolOutput {
                text: "nested failure".into(),
                is_error: true,
                exit_code: None
            }
        );
        assert_eq!(
            codex_tool_output(Some(&Value::String(
                "Process exited with code 7\ncommand failed".into()
            ))),
            CodexToolOutput {
                text: "Process exited with code 7\ncommand failed".into(),
                is_error: true,
                exit_code: Some(7),
            }
        );
    }

    #[test]
    fn codex_adapter_matches_meta_ids_and_maps_function_calls() {
        let store = tempfile::tempdir().unwrap();
        let dir = store.path().join("sessions/2026/08/28");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-opaque-name.jsonl");
        std::fs::write(
            &path,
            [
                r#"{"type":"session_meta","payload":{"id":"meta1234-0000-0000-0000-000000000000","cwd":"E:\\projects\\demo","thread_source":"user"}}"#,
                r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"run tests"}]}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call","call_id":"fn-1","name":"shell","arguments":"{\"cmd\":\"cargo test\"}"}}"#,
                r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"fn-1","output":"{\"exit_code\":1,\"output\":\"failed\"}"}}"#,
                r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let adapter = CodexAdapter::discover(store.path());

        let matches = adapter.matching("meta1234", false);
        let parsed = adapter.parse(&path).unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].session_id,
            "meta1234-0000-0000-0000-000000000000"
        );
        assert!(matches!(
            &parsed.records[1].kind,
            crate::session::RecordKind::Assistant(blocks)
                if matches!(&blocks[0], crate::session::AssistantBlock::ToolUse { name, .. } if name == "shell")
        ));
        assert!(matches!(
            &parsed.records[2].kind,
            crate::session::RecordKind::UserBlocks(blocks)
                if matches!(&blocks[0], crate::session::UserBlock::ToolResult { is_error: true, .. })
        ));
    }

    #[test]
    fn codex_adapter_reads_nested_subagent_nickname_fallback() {
        let store = tempfile::tempdir().unwrap();
        let dir = store.path().join("sessions/2026/08/28");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("rollout-worker.jsonl"),
            r#"{"type":"session_meta","payload":{"id":"worker12-0000-0000-0000-000000000000","thread_source":"subagent","source":{"subagent":{"thread_spawn":{"agent_nickname":"nested-worker"}}}}}"#,
        )
        .unwrap();
        let adapter = CodexAdapter::discover(store.path());

        let sessions = adapter.enumerate(true);

        assert_eq!(sessions[0].subagent.as_deref(), Some("nested-worker"));
    }
}
