//! Harness adapters and Store aggregation (ADR 0010).

use std::path::{Path, PathBuf};

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

#[derive(Clone)]
pub struct SessionHandle {
    pub info: SessionInfo,
    adapter: usize,
}

pub struct Stores {
    adapters: Vec<Box<dyn HarnessAdapter>>,
}

impl Stores {
    pub fn empty() -> Self {
        Self { adapters: Vec::new() }
    }

    pub fn with_claude(claude_dir: &Path) -> Self {
        Self { adapters: vec![Box::new(ClaudeAdapter::discover(claude_dir))] }
    }

    pub(crate) fn sessions(&self, scope: &Scope, include_subagents: bool) -> Vec<SessionHandle> {
        let mut sessions = Vec::new();
        for (adapter, source) in self.adapters.iter().enumerate() {
            sessions.extend(
                source
                    .enumerate(include_subagents)
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

    pub(crate) fn all_sessions(&self, include_subagents: bool) -> Vec<SessionHandle> {
        self.sessions(&Scope::All, include_subagents)
    }

    pub(crate) fn parse(&self, session: &SessionHandle) -> Option<Session> {
        self.adapters.get(session.adapter)?.parse(&session.info.path)
    }

    pub(crate) fn harness(&self, session: &SessionHandle) -> Harness {
        self.adapters[session.adapter].harness()
    }

    pub fn session_for_path(&self, path: &Path, include_subagents: bool) -> Option<SessionHandle> {
        self.all_sessions(include_subagents)
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
}
