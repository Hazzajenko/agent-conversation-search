//! The OpenCode adapter. OpenCode keeps every Session in one SQLite database,
//! `opencode.db`, in its Store (ADR 0016).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::harness::{Harness, HarnessAdapter, SessionLocator};
use crate::session::{Session, SessionMeta};
use crate::{ProjectKey, SessionIdentity};

/// The prefix of every OpenCode Session id. Display drops it (ADR 0017).
const ID_PREFIX: &str = "ses_";

pub(crate) struct OpenCodeAdapter {
    /// The `opencode.db` file, or `None` when the Store is missing.
    database: Option<PathBuf>,
}

impl OpenCodeAdapter {
    /// A directory without `opencode.db` is a missing Store. It lists nothing.
    pub(crate) fn discover(opencode_dir: &Path) -> Self {
        let database = opencode_dir.join("opencode.db");
        Self {
            database: database.is_file().then_some(database),
        }
    }

    /// Open the database read-only. The `immutable` flag is not set, so rows
    /// that OpenCode has committed to the WAL are visible.
    fn open(&self) -> Option<(&Path, Connection)> {
        let path = self.database.as_deref()?;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        Some((path, Connection::open_with_flags(path, flags).ok()?))
    }

    /// Every `session` row, child sessions included. A read failure lists
    /// nothing, the same as a missing Store.
    fn rows(&self) -> Vec<SessionRow> {
        let Some((path, db)) = self.open() else {
            return Vec::new();
        };
        let Ok(mut query) =
            db.prepare("SELECT id, parent_id, directory, title, time_updated FROM session")
        else {
            return Vec::new();
        };
        let Ok(rows) = query.query_map([], |row| {
            Ok(SessionRow {
                database: path.to_path_buf(),
                id: row.get(0)?,
                parent_id: row.get(1)?,
                directory: row.get(2)?,
                title: row.get(3)?,
                time_updated: row.get(4)?,
            })
        }) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    fn sessions(&self, prefix: Option<&str>, include_subagents: bool) -> Vec<SessionIdentity> {
        self.rows()
            .into_iter()
            .filter(|row| include_subagents || row.parent_id.is_none())
            .filter(|row| prefix.is_none_or(|prefix| id_has_prefix(&row.id, prefix)))
            .map(SessionRow::into_identity)
            .collect()
    }
}

impl HarnessAdapter for OpenCodeAdapter {
    fn harness(&self) -> Harness {
        Harness::OpenCode
    }

    fn recognizes(&self, _text: &str) -> bool {
        false
    }

    fn parse_text(&self, _text: &str) -> Option<Session> {
        None
    }

    fn enumerate(&self, include_subagents: bool) -> Vec<SessionIdentity> {
        self.sessions(None, include_subagents)
    }

    fn matching(&self, prefix: &str, include_subagents: bool) -> Vec<SessionIdentity> {
        self.sessions(Some(prefix), include_subagents)
    }

    fn session_ids(&self) -> Vec<String> {
        self.rows().into_iter().map(|row| row.id).collect()
    }

    /// The Session metadata. Messages are not read yet, so the Session has no
    /// Records.
    fn parse(&self, locator: &SessionLocator) -> Option<Session> {
        let SessionLocator::Database { session_id, .. } = locator else {
            return None;
        };
        let row = self.rows().into_iter().find(|row| &row.id == session_id)?;
        let identity = row.into_identity();
        Some(Session {
            meta: SessionMeta {
                title: identity.title,
                timestamp: identity.timestamp,
                cwd: identity.cwd,
                ..SessionMeta::default()
            },
            records: Vec::new(),
        })
    }
}

/// The `session` columns the adapter reads.
struct SessionRow {
    database: PathBuf,
    id: String,
    parent_id: Option<String>,
    directory: String,
    title: String,
    /// Epoch milliseconds.
    time_updated: i64,
}

impl SessionRow {
    fn into_identity(self) -> SessionIdentity {
        SessionIdentity {
            harness: Harness::OpenCode,
            subagent: self.parent_id.is_some().then(|| "worker".to_string()),
            project: Some(ProjectKey::from_cwd(&self.directory)),
            locator: SessionLocator::Database {
                path: self.database,
                session_id: self.id.clone(),
            },
            session_id: self.id,
            title: (!is_placeholder_title(&self.title)).then_some(self.title),
            timestamp: Some(iso8601_from_millis(self.time_updated)),
            branch: None,
            cwd: Some(self.directory),
            parent_id: self.parent_id,
        }
    }
}

/// Whether `id` starts with `prefix`. The prefix may leave out `ses_`.
pub(crate) fn id_has_prefix(id: &str, prefix: &str) -> bool {
    id.starts_with(prefix) || without_id_prefix(id).starts_with(prefix)
}

/// A Session id without the OpenCode `ses_` prefix. Other ids are unchanged.
pub(crate) fn without_id_prefix(id: &str) -> &str {
    id.strip_prefix(ID_PREFIX).unwrap_or(id)
}

/// Whether `title` is the title OpenCode gives a Session before it names it:
/// "New session - " or "Child session - " followed by the creation time.
fn is_placeholder_title(title: &str) -> bool {
    let Some(time) = ["New session - ", "Child session - "]
        .iter()
        .find_map(|prefix| title.strip_prefix(prefix))
    else {
        return false;
    };
    // Each `0` stands for one digit.
    const SHAPE: &[u8] = b"0000-00-00T00:00:00.000Z";
    time.len() == SHAPE.len()
        && time.bytes().zip(SHAPE).all(|(byte, &shape)| match shape {
            b'0' => byte.is_ascii_digit(),
            _ => byte == shape,
        })
}

/// Epoch milliseconds as ISO 8601 with milliseconds, the timestamp form
/// Claude Code writes, so recency sorts the same across Harnesses.
fn iso8601_from_millis(millis: i64) -> String {
    let seconds = crate::unix_to_iso8601_utc(millis.div_euclid(1000));
    format!(
        "{}.{:03}Z",
        seconds.trim_end_matches('Z'),
        millis.rem_euclid(1000)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_placeholder_titles_are_recognized() {
        assert!(is_placeholder_title(
            "New session - 2026-09-26T01:02:03.456Z"
        ));
        assert!(is_placeholder_title(
            "Child session - 2026-09-26T01:02:03.456Z"
        ));
        assert!(!is_placeholder_title("New session - fix the parser"));
        assert!(!is_placeholder_title("Fix the parser"));
    }

    #[test]
    fn milliseconds_render_as_iso_8601() {
        assert_eq!(
            iso8601_from_millis(1_787_911_200_123),
            "2026-08-28T10:00:00.123Z"
        );
    }

    #[test]
    fn a_prefix_matches_with_or_without_ses() {
        assert!(id_has_prefix("ses_f36c0fcf", "ses_f36c"));
        assert!(id_has_prefix("ses_f36c0fcf", "f36c"));
        assert!(!id_has_prefix("ses_f36c0fcf", "36c"));
    }

    #[test]
    fn the_adapter_enumerates_session_rows() {
        let store = tempfile::tempdir().unwrap();
        let db = Connection::open(store.path().join("opencode.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE session (id text PRIMARY KEY, parent_id text, directory text NOT NULL,
                title text NOT NULL, time_updated integer NOT NULL, time_archived integer);
             INSERT INTO session VALUES
                ('ses_parent', NULL, 'E:/projects/demo', 'Parent', 1787911200000, 1787911200000),
                ('ses_child', 'ses_parent', 'E:/projects/demo', 'Child', 1787911200000, NULL);",
        )
        .unwrap();
        drop(db);
        let adapter = OpenCodeAdapter::discover(store.path());

        let parents = adapter.enumerate(false);
        let all = adapter.enumerate(true);

        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].session_id, "ses_parent");
        assert_eq!(
            parents[0].project.as_ref().map(ProjectKey::as_str),
            Some("E--projects-demo")
        );
        assert_eq!(parents[0].title.as_deref(), Some("Parent"));
        assert_eq!(all.len(), 2);
        assert_eq!(
            all.iter()
                .find(|session| session.session_id == "ses_child")
                .and_then(|session| session.parent_id.as_deref()),
            Some("ses_parent")
        );
    }

    #[test]
    fn a_directory_without_a_database_lists_nothing() {
        let store = tempfile::tempdir().unwrap();

        assert!(OpenCodeAdapter::discover(store.path())
            .enumerate(true)
            .is_empty());
    }
}
