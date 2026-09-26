//! The OpenCode adapter. OpenCode keeps every Session in one SQLite database,
//! `opencode.db`, in its Store (ADR 0016).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::harness::{Harness, HarnessAdapter, SessionLocator};
use crate::session::{
    AssistantBlock, Record, RecordBuilder, RecordKind, Session, SessionMeta, UserBlock,
};
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

    /// The adapter for one database file, as a `show -` locator names it.
    pub(crate) fn for_database(database: &Path) -> Self {
        Self {
            database: database.is_file().then(|| database.to_path_buf()),
        }
    }

    /// Every `session` row, child sessions included. A read failure lists
    /// nothing, the same as a missing Store.
    fn rows(&self) -> Vec<SessionRow> {
        let Some((path, db)) = self.open() else {
            return Vec::new();
        };
        let Ok(mut query) = db.prepare(&format!("{SESSION_COLUMNS} FROM session")) else {
            return Vec::new();
        };
        let Ok(rows) = query.query_map([], |row| SessionRow::read(path, row)) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    /// The `session` row with the exact id `session_id`.
    fn row(&self, db: &Connection, session_id: &str) -> Option<SessionRow> {
        let path = self.database.as_deref()?;
        db.query_row(
            &format!("{SESSION_COLUMNS} FROM session WHERE id = ?1"),
            [session_id],
            |row| SessionRow::read(path, row),
        )
        .ok()
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

    /// The Session metadata from its `session` row and one Record per
    /// `message` row (ADR 0016).
    fn parse(&self, locator: &SessionLocator) -> Option<Session> {
        let SessionLocator::Database { session_id, .. } = locator else {
            return None;
        };
        let (_, db) = self.open()?;
        let identity = self.row(&db, session_id)?.into_identity();
        Some(Session {
            meta: SessionMeta {
                title: identity.title,
                timestamp: identity.timestamp,
                cwd: identity.cwd,
                ..SessionMeta::default()
            },
            records: read_records(&db, session_id)?,
        })
    }
}

/// The Records of one Session, in turn order. Each `message` row is one
/// Message. Its `text`, `reasoning`, and `tool` parts become Blocks, and the
/// other part kinds are noise.
fn read_records(db: &Connection, session_id: &str) -> Option<Vec<Record>> {
    let mut query = db
        .prepare(
            "SELECT m.id, m.time_created, m.data, p.data
             FROM message m LEFT JOIN part p ON p.message_id = m.id
             WHERE m.session_id = ?1
             ORDER BY m.time_created, m.id, p.id",
        )
        .ok()?;
    let rows = query
        .query_map([session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .ok()?;
    let mut messages: Vec<MessageRow> = Vec::new();
    for (id, time_created, data, part) in rows.flatten() {
        if messages.last().is_none_or(|message| message.id != id) {
            messages.push(MessageRow {
                id,
                time_created,
                data: serde_json::from_str(&data).unwrap_or(Value::Null),
                parts: Vec::new(),
            });
        }
        if let Some(part) = part.and_then(|data| serde_json::from_str(&data).ok()) {
            messages.last_mut()?.parts.push(part);
        }
    }
    let mut records = RecordBuilder::default();
    for message in &messages {
        let timestamp = Some(iso8601_from_millis(message.time_created));
        match message.data.get("role").and_then(Value::as_str) {
            Some("user") => records.message(user_kind(&message.parts), timestamp, None),
            Some("assistant") => {
                let (blocks, results) = assistant_blocks(&message.parts);
                records.message(Some(RecordKind::Assistant(blocks)), timestamp.clone(), None);
                if !results.is_empty() {
                    records.same_message(Some(RecordKind::UserBlocks(results)), timestamp, None);
                }
            }
            _ => {}
        }
    }
    Some(records.finish())
}

/// One `message` row with its parts, in part order.
struct MessageRow {
    id: String,
    /// Epoch milliseconds.
    time_created: i64,
    data: Value,
    parts: Vec<Value>,
}

/// The Prompt of a user Message is its `text` parts joined. OpenCode marks text
/// it adds itself, such as file contents, as `synthetic`. That text is not
/// what the person typed, so it is left out.
fn user_kind(parts: &[Value]) -> Option<RecordKind> {
    let texts: Vec<&str> = parts
        .iter()
        .filter(|part| part_type(part) == Some("text"))
        .filter(|part| !flag(part, "synthetic") && !flag(part, "ignored"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    (!texts.is_empty()).then(|| RecordKind::Prompt(texts.join("\n")))
}

/// The Blocks of an assistant Message and the tool results it holds. A `tool`
/// part is both the call and, once it has finished, its result.
fn assistant_blocks(parts: &[Value]) -> (Vec<AssistantBlock>, Vec<UserBlock>) {
    let mut blocks = Vec::new();
    let mut results = Vec::new();
    for part in parts {
        let text = || {
            part.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        match part_type(part) {
            Some("text") => blocks.push(AssistantBlock::Text(text())),
            Some("reasoning") => blocks.push(AssistantBlock::Thinking(text())),
            Some("tool") => {
                let id = part
                    .get("callID")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let state = part.get("state");
                let field = |key: &str| {
                    state
                        .and_then(|state| state.get(key))
                        .and_then(Value::as_str)
                };
                blocks.push(AssistantBlock::ToolUse {
                    id: id.clone(),
                    name: part
                        .get("tool")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    input: state
                        .and_then(|state| state.get("input"))
                        .cloned()
                        .unwrap_or(Value::Null),
                });
                let (is_error, text) = match field("status") {
                    Some("completed") => (false, field("output")),
                    Some("error") => (true, field("error")),
                    _ => continue,
                };
                results.push(UserBlock::ToolResult {
                    is_error,
                    exit_code: None,
                    tool_use_id: id,
                    text: text.unwrap_or_default().to_string(),
                });
            }
            _ => {}
        }
    }
    (blocks, results)
}

fn part_type(part: &Value) -> Option<&str> {
    part.get("type").and_then(Value::as_str)
}

fn flag(part: &Value, key: &str) -> bool {
    part.get(key).and_then(Value::as_bool) == Some(true)
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

/// The `SELECT` list that [`SessionRow::read`] expects.
const SESSION_COLUMNS: &str = "SELECT id, parent_id, directory, title, time_updated";

impl SessionRow {
    fn read(database: &Path, row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            database: database.to_path_buf(),
            id: row.get(0)?,
            parent_id: row.get(1)?,
            directory: row.get(2)?,
            title: row.get(3)?,
            time_updated: row.get(4)?,
        })
    }

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
    fn the_adapter_reads_one_record_per_message() {
        let store = tempfile::tempdir().unwrap();
        let db = Connection::open(store.path().join("opencode.db")).unwrap();
        db.execute_batch(
            r#"CREATE TABLE session (id text PRIMARY KEY, parent_id text, directory text NOT NULL,
                title text NOT NULL, time_updated integer NOT NULL);
             CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL,
                time_created integer NOT NULL, data text NOT NULL);
             CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL,
                data text NOT NULL);
             INSERT INTO session VALUES ('ses_a', NULL, 'E:/projects/demo', 'Demo', 1787911200000);
             INSERT INTO message VALUES
                ('msg_1', 'ses_a', 1787911200000, '{"role":"user"}'),
                ('msg_2', 'ses_a', 1787911201000, '{"role":"assistant"}'),
                ('msg_3', 'ses_a', 1787911202000, '{"role":"user"}');
             INSERT INTO part VALUES
                ('prt_1', 'msg_1', '{"type":"text","text":"typed"}'),
                ('prt_2', 'msg_1', '{"type":"text","text":"file body","synthetic":true}'),
                ('prt_3', 'msg_2', '{"type":"step-start"}'),
                ('prt_4', 'msg_2', '{"type":"tool","tool":"read","callID":"c1","state":{"status":"error","input":{"filePath":"a.rs"},"error":"missing"}}'),
                ('prt_5', 'msg_2', '{"type":"tool","tool":"bash","callID":"c2","state":{"status":"running","input":{"command":"ls"}}}'),
                ('prt_6', 'msg_2', '{"type":"unknown-kind","text":"ignored"}'),
                ('prt_7', 'msg_2', '{"type":"text","text":"reply"}');"#,
        )
        .unwrap();
        drop(db);
        let adapter = OpenCodeAdapter::discover(store.path());

        let session = adapter
            .parse(&SessionLocator::Database {
                path: store.path().join("opencode.db"),
                session_id: "ses_a".into(),
            })
            .unwrap();

        let turns: Vec<_> = session.records.iter().map(|record| record.turn).collect();
        assert_eq!(turns, vec![Some(1), Some(2), Some(2)]);
        assert_eq!(session.records[0].kind, RecordKind::Prompt("typed".into()));
        let RecordKind::Assistant(blocks) = &session.records[1].kind else {
            panic!("expected an assistant Record");
        };
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[2], AssistantBlock::Text("reply".into()));
        assert_eq!(
            session.records[2].kind,
            RecordKind::UserBlocks(vec![UserBlock::ToolResult {
                is_error: true,
                exit_code: None,
                tool_use_id: Some("c1".into()),
                text: "missing".into(),
            }])
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
