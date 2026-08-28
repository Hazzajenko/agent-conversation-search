//! Harness adapters and Store aggregation (ADR 0010).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::session::Session;
use crate::{Scope, SessionInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Codex,
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
    fn enumerate(&self, include_subagents: bool) -> Vec<SessionInfo>;
    fn parse(&self, path: &Path) -> Option<Session>;
}

pub(crate) struct ClaudeAdapter {
    projects_root: PathBuf,
}

impl ClaudeAdapter {
    pub(crate) fn discover(claude_dir: &Path) -> Self {
        Self { projects_root: claude_dir.join("projects") }
    }
}

impl HarnessAdapter for ClaudeAdapter {
    fn harness(&self) -> Harness {
        Harness::Claude
    }

    fn enumerate(&self, _include_subagents: bool) -> Vec<SessionInfo> {
        let Ok(projects) = std::fs::read_dir(&self.projects_root) else {
            return Vec::new();
        };
        let mut sessions = Vec::new();
        for project in projects.flatten().filter(|entry| entry.file_type().is_ok_and(|ty| ty.is_dir())) {
            let project_name = project.file_name().to_string_lossy().into_owned();
            let Ok(files) = std::fs::read_dir(project.path()) else {
                continue;
            };
            for path in files.flatten().map(|entry| entry.path()) {
                if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                    continue;
                }
                let session_id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                if session_id.is_empty() {
                    continue;
                }
                let Some(parsed) = self.parse(&path) else {
                    continue;
                };
                sessions.push(SessionInfo {
                    harness: Harness::Claude,
                    subagent: None,
                    project: project_name.clone(),
                    session_id,
                    path,
                    title: parsed.meta.title,
                    timestamp: parsed.meta.timestamp,
                    branch: parsed.meta.branch,
                    cwd: parsed.meta.cwd,
                });
            }
        }
        sessions
    }

    fn parse(&self, path: &Path) -> Option<Session> {
        std::fs::read_to_string(path).ok().map(|text| crate::session::read(&text))
    }
}

pub(crate) struct CodexAdapter {
    codex_dir: PathBuf,
}

impl CodexAdapter {
    pub(crate) fn discover(codex_dir: &Path) -> Self {
        Self { codex_dir: codex_dir.to_path_buf() }
    }

    fn rollout_paths(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        visit_jsonl(&self.codex_dir.join("sessions"), &mut files);
        files.sort();
        files
    }
}

impl HarnessAdapter for CodexAdapter {
    fn harness(&self) -> Harness {
        Harness::Codex
    }

    fn enumerate(&self, include_subagents: bool) -> Vec<SessionInfo> {
        let titles = codex_titles(&self.codex_dir.join("session_index.jsonl"));
        self.rollout_paths()
            .into_iter()
            .filter_map(|path| {
                let text = std::fs::read_to_string(&path).ok()?;
                let first = text.lines().next()?;
                let meta: Value = serde_json::from_str(first).ok()?;
                if meta.get("type").and_then(Value::as_str) != Some("session_meta") {
                    return None;
                }
                let thread_source = meta.pointer("/payload/thread_source").and_then(Value::as_str);
                if thread_source == Some("subagent") && !include_subagents {
                    return None;
                }
                let session_id = meta
                    .pointer("/payload/id")
                    .or_else(|| meta.pointer("/payload/session_id"))
                    .and_then(Value::as_str)?
                    .to_string();
                let title = titles.get(&session_id).cloned();
                let subagent = if thread_source == Some("subagent") {
                    Some(
                        meta.pointer("/payload/source/subagent/other")
                            .and_then(Value::as_str)
                            .unwrap_or("worker")
                            .to_string(),
                    )
                } else {
                    None
                };
                let cwd = meta.pointer("/payload/cwd").and_then(Value::as_str).map(str::to_string);
                let parsed = codex_read(&text);
                Some(SessionInfo {
                    harness: Harness::Codex,
                    subagent,
                    project: cwd.as_deref().map(crate::encode_project_dir).unwrap_or_default(),
                    session_id,
                    path,
                    title,
                    timestamp: parsed.meta.timestamp,
                    branch: parsed.meta.branch,
                    cwd,
                })
            })
            .collect()
    }

    fn parse(&self, path: &Path) -> Option<Session> {
        std::fs::read_to_string(path).ok().map(|text| codex_read(&text))
    }
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

fn codex_read(text: &str) -> Session {
    let mut meta = crate::session::SessionMeta::default();
    let mut records = Vec::new();
    let mut turn = 0;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(timestamp) = value.get("timestamp").and_then(Value::as_str) {
            if meta.timestamp.as_deref().is_none_or(|current| timestamp > current) {
                meta.timestamp = Some(timestamp.to_string());
            }
        }
        if meta.cwd.is_none() {
            meta.cwd = value.pointer("/payload/cwd").and_then(Value::as_str).map(str::to_string);
        }
        if meta.branch.is_none() {
            meta.branch = value.pointer("/payload/git/branch").and_then(Value::as_str).map(str::to_string);
        }
        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let Some(kind) = codex_record_kind(payload) else {
            continue;
        };
        turn += 1;
        records.push(crate::session::Record { turn: Some(turn), kind });
    }
    Session { meta, records }
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
                Some("user") => Some(crate::session::RecordKind::Prompt(texts.collect::<Vec<_>>().join("\n"))),
                Some("assistant") => Some(crate::session::RecordKind::Assistant(
                    texts.map(|text| crate::session::AssistantBlock::Text(text.to_string())).collect(),
                )),
                _ => None,
            }
        }
        Some("reasoning") => {
            let text = payload
                .get("summary")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            Some(crate::session::RecordKind::Assistant(vec![
                crate::session::AssistantBlock::Thinking(text),
            ]))
        }
        Some("custom_tool_call") => {
            let id = payload.get("call_id").and_then(Value::as_str).map(str::to_string);
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();
            let input = payload
                .get("input")
                .and_then(Value::as_str)
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or(Value::Null);
            Some(crate::session::RecordKind::Assistant(vec![
                crate::session::AssistantBlock::ToolUse { id, name, input },
            ]))
        }
        Some("custom_tool_call_output") => {
            let tool_use_id = payload.get("call_id").and_then(Value::as_str).map(str::to_string);
            let text = codex_output_text(payload.get("output"));
            Some(crate::session::RecordKind::UserBlocks(vec![
                crate::session::UserBlock::ToolResult { is_error: false, tool_use_id, text },
            ]))
        }
        _ => None,
    }
}

fn codex_output_text(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[derive(Clone)]
pub struct SessionHandle {
    pub info: SessionInfo,
    adapter: usize,
}

pub struct Stores {
    adapters: Vec<Box<dyn HarnessAdapter>>,
    include_subagents: bool,
}

impl Stores {
    pub fn empty() -> Self {
        Self { adapters: Vec::new(), include_subagents: false }
    }

    pub fn with_claude(claude_dir: &Path) -> Self {
        Self { adapters: vec![Box::new(ClaudeAdapter::discover(claude_dir))], include_subagents: false }
    }

    pub fn with_codex(codex_dir: &Path) -> Self {
        Self { adapters: vec![Box::new(CodexAdapter::discover(codex_dir))], include_subagents: false }
    }

    pub fn with_claude_and_codex(claude_dir: &Path, codex_dir: &Path) -> Self {
        Self {
            adapters: vec![
                Box::new(ClaudeAdapter::discover(claude_dir)),
                Box::new(CodexAdapter::discover(codex_dir)),
            ],
            include_subagents: false,
        }
    }

    pub fn including_subagents(mut self, include: bool) -> Self {
        self.include_subagents = include;
        self
    }

    pub(crate) fn sessions(&self, scope: &Scope) -> Vec<SessionHandle> {
        let mut sessions = Vec::new();
        for (adapter, source) in self.adapters.iter().enumerate() {
            sessions.extend(
                source
                    .enumerate(self.include_subagents)
                    .into_iter()
                    .filter(|info| in_scope(info, scope))
                    .map(|info| SessionHandle { info, adapter }),
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

    pub(crate) fn all_sessions(&self) -> Vec<SessionHandle> {
        self.sessions(&Scope::All)
    }

    pub(crate) fn parse(&self, session: &SessionHandle) -> Option<Session> {
        self.adapters.get(session.adapter)?.parse(&session.info.path)
    }

    pub(crate) fn harness(&self, session: &SessionHandle) -> Harness {
        self.adapters[session.adapter].harness()
    }

    pub fn session_for_path(&self, path: &Path) -> Option<SessionHandle> {
        self.all_sessions()
            .into_iter()
            .find(|session| session.info.path == path)
    }
}

fn in_scope(info: &SessionInfo, scope: &Scope) -> bool {
    match scope {
        Scope::All => true,
        Scope::Current { cwd } => info.project.eq_ignore_ascii_case(&crate::encode_project_dir(cwd)),
        Scope::Project { name_substring } => info
            .project
            .to_lowercase()
            .contains(&name_substring.to_lowercase()),
    }
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
        assert_eq!(sessions[0].project, "E--projects-demo");
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
        assert_eq!(sessions[0].session_id, "c0de0001-0000-0000-0000-000000000000");
        assert_eq!(sessions[0].project, "E--projects-demo");
        assert_eq!(sessions[0].title.as_deref(), Some("Indexed title"));
        assert_eq!(parsed.records.len(), 2);
        assert_eq!(parsed.records[0].kind, crate::session::RecordKind::Prompt("prompt".into()));
        assert!(matches!(
            &parsed.records[1].kind,
            crate::session::RecordKind::Assistant(blocks)
                if blocks == &vec![crate::session::AssistantBlock::Text("reply".into())]
        ));
    }
}
