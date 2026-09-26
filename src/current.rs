//! Current Session resolution (ADR 0013, ADR 0014).
//!
//! One module owns ambient identity detection, Store lookup, ancestry, and
//! cross-Harness ambiguity. Commands never read Harness environment variables
//! or reconstruct a Session family themselves.

use std::collections::{HashMap, HashSet};

use crate::harness::SessionKey;
use crate::{Harness, SessionHandle, SessionIdentity, Stores};

/// The resolved Current Session, the calling thread, and the IDs that form
/// the Current Session Family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentContext {
    pub session: SessionIdentity,
    pub thread: SessionIdentity,
    pub family_ids: Vec<String>,
}

impl CurrentContext {
    pub fn caller_is_worker(&self) -> bool {
        self.thread.session_id != self.session.session_id
    }
}

/// Why current-context resolution failed. Explicit current-context commands
/// treat every variant as a nonzero error; they never fall back to recency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurrentContextError {
    Unavailable,
    NotInStore {
        harness: Harness,
        session_id: String,
    },
    Ambiguous {
        harnesses: Vec<Harness>,
    },
}

impl std::fmt::Display for CurrentContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => write!(f, "no current Session; run from a supported Harness"),
            Self::NotInStore {
                harness,
                session_id,
            } => write!(
                f,
                "current Session {session_id} was not found in the configured {} Store",
                harness.as_str()
            ),
            Self::Ambiguous { harnesses } => {
                let names = harnesses
                    .iter()
                    .map(|harness| harness.as_str())
                    .collect::<Vec<_>>()
                    .join(" and ");
                write!(
                    f,
                    "current Session is ambiguous between {names}; pass --harness claude or --harness codex"
                )
            }
        }
    }
}

struct AmbientIdentity {
    claude_session_id: Option<String>,
    codex_session_id: Option<String>,
    codex_thread_id: Option<String>,
}

impl AmbientIdentity {
    fn from_env() -> Self {
        Self {
            claude_session_id: env_id("CLAUDE_CODE_SESSION_ID"),
            codex_session_id: env_id("CODEX_SESSION_ID"),
            codex_thread_id: env_id("CODEX_THREAD_ID"),
        }
    }

    fn claude_present(&self) -> bool {
        self.claude_session_id.is_some()
    }

    fn codex_present(&self) -> bool {
        self.codex_session_id.is_some() || self.codex_thread_id.is_some()
    }
}

fn env_id(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Resolve the Current Session from ambient Harness identity and the
/// configured Stores. Never infers identity from recency.
pub fn resolve_current_context(stores: &Stores) -> Result<CurrentContext, CurrentContextError> {
    resolve(stores, AmbientIdentity::from_env())
}

/// Resolve the Current Session to its exact Store handle.
pub fn resolve_current_session(stores: &Stores) -> Result<SessionHandle, CurrentContextError> {
    let context = resolve_current_context(stores)?;
    stores
        .lookup_session(context.session.harness, &context.session.session_id)
        .ok_or(CurrentContextError::NotInStore {
            harness: context.session.harness,
            session_id: context.session.session_id,
        })
}

/// Resolve the calling thread to its exact Store handle.
///
/// At the top level this identifies the same Session as
/// [`resolve_current_session`]; from a spawned worker it identifies the
/// worker's own subagent thread. Like [`resolve_current_session`] the lookup
/// includes subagent threads, so an explicit `current-thread` selector can
/// access a worker without `--include-subagents`.
pub fn resolve_current_thread(stores: &Stores) -> Result<SessionHandle, CurrentContextError> {
    let context = resolve_current_context(stores)?;
    stores
        .lookup_session(context.thread.harness, &context.thread.session_id)
        .ok_or(CurrentContextError::NotInStore {
            harness: context.thread.harness,
            session_id: context.thread.session_id,
        })
}

/// The Sessions multi-Session analysis excludes by default (ADR 0011): the
/// Current Session and every known descendant. Each key includes its Harness
/// because Session ids can coincide across Stores.
///
/// Unlike [`resolve_current_context`] this never fails — analysis outside a
/// Harness, or with an identity absent from the Stores, keeps its existing
/// behavior. Two partial cases resolve to more exclusion rather than less,
/// because a half-known family is still current work:
///
/// * an ancestor missing from the Store falls back to the calling thread and
///   its descendants, so a pruned parent cannot leave the thread searchable;
/// * cross-Harness ambiguity excludes both families.
pub(crate) fn current_family_keys(stores: &Stores) -> HashSet<SessionKey> {
    harness_resolutions(stores, AmbientIdentity::from_env())
        .into_iter()
        .filter_map(|resolution| resolution.family(stores))
        .flatten()
        .collect()
}

/// One Harness's current-context result and the calling thread used when only
/// part of that context resolves.
struct HarnessResolution {
    thread: Option<SessionIdentity>,
    context: Result<CurrentContext, CurrentContextError>,
}

impl HarnessResolution {
    /// The keys this resolution contributes to the exclusion set, falling back to
    /// the calling thread's family when only the ancestry was unresolvable.
    fn family(self, stores: &Stores) -> Option<Vec<SessionKey>> {
        match self.context {
            Ok(context) => Some(qualify_family(context.session.harness, context.family_ids)),
            Err(_) => self.thread.map(|thread| {
                qualify_family(
                    thread.harness,
                    collect_family(stores, thread.harness, &thread.session_id),
                )
            }),
        }
    }
}

fn qualify_family(harness: Harness, session_ids: Vec<String>) -> Vec<SessionKey> {
    session_ids
        .into_iter()
        .map(|session_id| SessionKey {
            harness,
            session_id,
        })
        .collect()
}

fn harness_resolutions(stores: &Stores, ambient: AmbientIdentity) -> Vec<HarnessResolution> {
    let configured = stores.configured_harnesses();
    let mut resolutions = Vec::new();
    if configured.contains(&Harness::Claude) && ambient.claude_present() {
        resolutions.push(resolve_claude(
            stores,
            ambient.claude_session_id.as_deref().unwrap(),
        ));
    }
    if configured.contains(&Harness::Codex) && ambient.codex_present() {
        resolutions.push(resolve_codex(stores, &ambient));
    }
    resolutions
}

fn resolve(
    stores: &Stores,
    ambient: AmbientIdentity,
) -> Result<CurrentContext, CurrentContextError> {
    let resolutions = harness_resolutions(stores, ambient);
    if resolutions.is_empty() {
        return Err(CurrentContextError::Unavailable);
    }

    let mut contexts = Vec::new();
    let mut errors = Vec::new();
    for resolution in resolutions {
        match resolution.context {
            Ok(context) => contexts.push(context),
            Err(error) => errors.push(error),
        }
    }

    match contexts.len() {
        0 => Err(errors.into_iter().next().unwrap()),
        1 => Ok(contexts.pop().unwrap()),
        _ => Err(CurrentContextError::Ambiguous {
            harnesses: contexts
                .into_iter()
                .map(|context| context.session.harness)
                .collect(),
        }),
    }
}

fn resolve_claude(stores: &Stores, session_id: &str) -> HarnessResolution {
    let thread = match lookup_required(stores, Harness::Claude, session_id) {
        Ok(thread) => thread,
        Err(error) => {
            return HarnessResolution {
                thread: None,
                context: Err(error),
            }
        }
    };
    let context =
        walk_to_root(stores, &thread).map(|session| context_from(stores, session, thread.clone()));
    HarnessResolution {
        thread: Some(thread),
        context,
    }
}

fn resolve_codex(stores: &Stores, ambient: &AmbientIdentity) -> HarnessResolution {
    let thread_id = ambient
        .codex_thread_id
        .as_deref()
        .or(ambient.codex_session_id.as_deref())
        .unwrap();
    let thread = match lookup_required(stores, Harness::Codex, thread_id) {
        Ok(thread) => thread,
        Err(error) => {
            return HarnessResolution {
                thread: None,
                context: Err(error),
            }
        }
    };
    // Codex names the top-level Session directly when it differs from the
    // calling thread; otherwise the thread's own ancestry is the only source.
    let session = match ambient.codex_session_id.as_deref() {
        Some(session_id) if session_id != thread_id => {
            lookup_required(stores, Harness::Codex, session_id)
        }
        _ => walk_to_root(stores, &thread),
    };
    let context = session.map(|session| context_from(stores, session, thread.clone()));
    HarnessResolution {
        thread: Some(thread),
        context,
    }
}

fn lookup_required(
    stores: &Stores,
    harness: Harness,
    session_id: &str,
) -> Result<SessionIdentity, CurrentContextError> {
    stores
        .lookup_session(harness, session_id)
        .map(|handle| handle.info)
        .ok_or(CurrentContextError::NotInStore {
            harness,
            session_id: session_id.to_string(),
        })
}

fn context_from(
    stores: &Stores,
    session: SessionIdentity,
    thread: SessionIdentity,
) -> CurrentContext {
    CurrentContext {
        family_ids: collect_family(stores, session.harness, &session.session_id),
        session,
        thread,
    }
}

fn walk_to_root(
    stores: &Stores,
    identity: &SessionIdentity,
) -> Result<SessionIdentity, CurrentContextError> {
    let mut current = identity.clone();
    let mut seen = HashSet::new();
    while let Some(parent_id) = current.parent_id.clone() {
        if !seen.insert(current.session_id.clone()) {
            break;
        }
        current = lookup_required(stores, identity.harness, &parent_id)?;
    }
    Ok(current)
}

fn collect_family(stores: &Stores, harness: Harness, root_id: &str) -> Vec<String> {
    let parents: HashMap<String, Option<String>> = stores
        .sessions_including_subagents(harness)
        .into_iter()
        .map(|session| {
            (
                session.info.session_id.clone(),
                session.info.parent_id.clone(),
            )
        })
        .collect();
    let mut family: Vec<String> = parents
        .keys()
        .filter(|id| id.as_str() == root_id || walks_to(id, root_id, &parents))
        .cloned()
        .collect();
    family.sort();
    family
}

fn walks_to(id: &str, root: &str, parents: &HashMap<String, Option<String>>) -> bool {
    crate::walks_to_ancestor(id, root, |current| {
        parents.get(current).and_then(|p| p.clone())
    })
}

/// Human-readable Current Session metadata for `agsearch current`.
pub fn format_current(context: &CurrentContext) -> String {
    let caller = if context.caller_is_worker() {
        "worker"
    } else {
        "top-level"
    };
    format!(
        "Harness: {}\nSession: {}\nProject: {}\nTitle: {}\nPath: {}\nCaller: {}\n",
        context.session.harness.as_str(),
        context.session.session_id,
        context.session.display_project(),
        context.session.title.as_deref().unwrap_or("(untitled)"),
        context.session.locator,
        caller
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn family_includes_the_current_session_and_its_worker() {
        let store = tempfile::tempdir().unwrap();
        let cwd = store.path().join("demo");
        let parent_id = "c0de2001-0000-0000-0000-000000000000";
        let worker_id = "c0de2002-0000-0000-0000-000000000000";
        let dir = store
            .path()
            .join("sessions")
            .join("2026")
            .join("08")
            .join("28");
        fs::create_dir_all(&dir).unwrap();
        let parent_meta = serde_json::json!({
            "timestamp": "2026-08-28T10:00:00.000Z",
            "type": "session_meta",
            "payload": {"id": parent_id, "cwd": cwd, "thread_source": "user"}
        });
        let worker_meta = serde_json::json!({
            "timestamp": "2026-08-28T10:00:00.000Z",
            "type": "session_meta",
            "payload": {
                "id": worker_id,
                "cwd": cwd,
                "thread_source": "subagent",
                "parent_thread_id": parent_id
            }
        });
        fs::write(
            dir.join(format!("rollout-{parent_id}.jsonl")),
            parent_meta.to_string(),
        )
        .unwrap();
        fs::write(
            dir.join(format!("rollout-{worker_id}.jsonl")),
            worker_meta.to_string(),
        )
        .unwrap();

        let stores = Stores::with_codex(store.path());
        let context = resolve(
            &stores,
            AmbientIdentity {
                claude_session_id: None,
                codex_session_id: Some(parent_id.into()),
                codex_thread_id: Some(worker_id.into()),
            },
        )
        .unwrap();

        assert_eq!(context.session.session_id, parent_id);
        assert_eq!(context.thread.session_id, worker_id);
        assert!(context.caller_is_worker());
        assert_eq!(
            context.family_ids,
            vec![parent_id.to_string(), worker_id.to_string()]
        );
    }
}
