//! The one parser for the on-disk Session format (ADR 0006).
//!
//! `read` turns a Session file's text into a typed [`Session`] — header
//! [`SessionMeta`] plus a sequence of turn-numbered [`Record`]s — in a single
//! pass. It is the *only* place that knows where things live in the JSONL: every
//! consumer (`search`, `--failed`, `show`, `sessions`) is a pure projection over
//! `&[Record]`.
//!
//! Two rules keep it deep:
//! - **Lossless.** The parser represents what is on disk, including empty blocks
//!   (a signature-only `Thinking("")`). It applies no content policy; each
//!   projection decides what to drop — because `search` and `show` legitimately
//!   disagree about empties (ADR 0006).
//! - **Turn numbering lives here, once.** The number ticks on every `user` /
//!   `assistant` Record, even a contentless one, so the find→read handoff
//!   coordinate (ADR 0002) holds by construction rather than by a cross-check.

use std::collections::HashMap;

use serde_json::Value;

/// Keys whose value, if present, is the most informative one-liner argument for a
/// tool call. Tried in order; the first string value wins. A fixed list (rather
/// than dumping raw JSON) is what makes a tool call readable.
const TOOL_ARG_KEYS: &[&str] = &[
    "command",
    "cmd",
    "file_path",
    "pattern",
    "path",
    "url",
    "query",
    "prompt",
];

/// The single most informative argument of a `tool_use` input object, or `None`
/// if it carries none of the known keys. Tool-format knowledge, so it lives with
/// the parser and is shared by every projection that renders a tool call.
pub(crate) fn tool_key_arg(input: &Value) -> Option<String> {
    TOOL_ARG_KEYS
        .iter()
        .find_map(|key| input.get(*key).and_then(|v| v.as_str()).map(str::to_string))
}

/// A Session's header metadata, accumulated in the same pass as its Records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SessionMeta {
    /// The Harness-derived Title, if any. Claude reads an `ai-title` Record;
    /// Codex reads `thread_name` from its Store index.
    pub title: Option<String>,
    /// The newest record timestamp (raw ISO 8601) — the recency key. ISO 8601
    /// sorts lexically, so the max string is the newest record.
    pub timestamp: Option<String>,
    /// The git branch (`gitBranch`), first one seen.
    pub branch: Option<String>,
    /// The real working directory (`cwd`), first one seen.
    pub cwd: Option<String>,
    /// The final cumulative Codex totals, for the usage-breakdown mismatch
    /// check (issue 31). Normalized like per-call Usage (non-cached input),
    /// model and call id unset. `None` when the Session has no `token_count`
    /// Records or the final one carries no totals.
    pub codex_final_total: Option<Usage>,
}

/// One Block of a user Message's `content` array. User array content is
/// `tool_result` (and occasionally `text`); a user Block can never be a
/// `tool_use` (ADR 0006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UserBlock {
    Text(String),
    ToolResult {
        is_error: bool,
        exit_code: Option<i64>,
        tool_use_id: Option<String>,
        text: String,
    },
}

/// One Block of an assistant Message's `content` array. An assistant Block can
/// never be a `tool_result` (ADR 0006). `ToolUse` keeps the raw `input` Value so
/// each projection can extract what it needs (the full serialized args for
/// search, the single key argument for `show` / `--failed`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AssistantBlock {
    Text(String),
    Thinking(String),
    ToolUse {
        id: Option<String>,
        name: String,
        input: Value,
    },
}

/// What a [`Record`] is. Only Message, Title, and Codex usage Records are
/// kept; noise Records (`queue-operation`, `mode`, `attachment`, unparseable
/// lines) are skipped — but the turn counter still ticks past every `user` /
/// `assistant` line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RecordKind {
    /// A user Message the person typed (string content).
    Prompt(String),
    /// A user Message carrying `tool_result` (and occasional `text`) blocks.
    UserBlocks(Vec<UserBlock>),
    /// An assistant Message's content blocks.
    Assistant(Vec<AssistantBlock>),
    /// An `ai-title` Record (session metadata, not a turn).
    Title(String),
    /// A Codex `token_count` event: one model call's token counts, no text.
    /// Carries [`Usage`] on the Record; ignored by search/show/Transcript
    /// (like Title), used only by the usage breakdown. Turn is always `None`
    /// (it never consumes a Transcript turn); the breakdown numbers Codex
    /// calls by file order instead.
    CodexTokenCount,
}

/// Token Usage for one model call (see CONTEXT.md Usage). Carried on each
/// assistant [`Record`] (Claude Code) or [`RecordKind::CodexTokenCount`]
/// Record (Codex), so every projection sees the same numbers. Claude Code
/// attaches it to each assistant Message (`message.usage` plus `message.model`
/// and `message.id`); Codex `token_count` Records carry `last_token_usage`
/// with the model from the session meta. Token counts are `None` when the
/// Harness recorded no Usage for the call — the breakdown then shows blank
/// numeric cells but still lists the call with its model and preview. Tokens
/// only, no money.
///
/// For Codex, `input` is the *non-cached* input (raw `input_tokens` minus
/// `cached_input_tokens` minus `cache_write_input_tokens`) so Codex and Claude
/// totals are comparable for cross-Harness ranking: both sum to
/// input + cache-write + cache-read + output, matching Codex `total_tokens`
/// (`input_tokens + output_tokens`, cached subsets). `cache_write` is
/// `Some(0)` when the file predates `cache_write_input_tokens`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Usage {
    pub input: Option<u64>,
    pub cache_create: Option<u64>,
    pub cache_read: Option<u64>,
    pub output: Option<u64>,
    pub model: Option<String>,
    /// The Harness's call id (`message.id` for Claude Code). Several Records
    /// sharing one id are one call and its Usage is counted once.
    pub call_id: Option<String>,
}

impl Usage {
    /// Total tokens, or `None` when the call carries no token counts.
    #[allow(dead_code)]
    pub fn total(&self) -> Option<u64> {
        match (self.input, self.cache_create, self.cache_read, self.output) {
            (Some(i), Some(c), Some(r), Some(o)) => {
                Some(i.saturating_add(c).saturating_add(r).saturating_add(o))
            }
            _ => None,
        }
    }
}

/// One parsed Record, tagged with its turn number (Message order, the ADR 0002
/// numbering). `None` for a Title — it is metadata, not a turn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Record {
    pub turn: Option<usize>,
    /// Raw ISO 8601 timestamp from the envelope, if present. Powers the usage
    /// breakdown's per-call timestamp column; recency filtering keeps using
    /// [`SessionMeta::timestamp`] (the max).
    pub timestamp: Option<String>,
    /// Token Usage for this call, if the Harness recorded it. `None` renders
    /// blank numeric cells in the usage breakdown but the call still appears.
    pub usage: Option<Usage>,
    pub kind: RecordKind,
}

/// A whole Session parsed from its file text: header metadata plus its
/// turn-numbered Records, in file order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Session {
    pub meta: SessionMeta,
    pub records: Vec<Record>,
}

/// Assign the shared Message-order coordinate while a Harness parser builds a
/// Session. `message` advances it; adapter-normalized Blocks use `attached` to
/// buffer until their following Message and receive that coordinate; metadata
/// has no coordinate. Timestamp and Usage travel with the kind so the usage
/// breakdown sees the same per-call data every projection shares.
#[derive(Default)]
pub(crate) struct RecordBuilder {
    turn: usize,
    records: Vec<Record>,
    attached: Vec<(RecordKind, Option<String>, Option<Usage>)>,
}

impl RecordBuilder {
    pub(crate) fn message(
        &mut self,
        kind: Option<RecordKind>,
        timestamp: Option<String>,
        usage: Option<Usage>,
    ) {
        self.turn += 1;
        for (kind, timestamp, usage) in self.attached.drain(..) {
            self.records.push(Record {
                turn: Some(self.turn),
                timestamp,
                usage,
                kind,
            });
        }
        self.push(kind, Some(self.turn), timestamp, usage);
    }

    pub(crate) fn attached(
        &mut self,
        kind: Option<RecordKind>,
        timestamp: Option<String>,
        usage: Option<Usage>,
    ) {
        if let Some(kind) = kind {
            self.attached.push((kind, timestamp, usage));
        }
    }

    pub(crate) fn metadata(
        &mut self,
        kind: Option<RecordKind>,
        timestamp: Option<String>,
        usage: Option<Usage>,
    ) {
        self.push(kind, None, timestamp, usage);
    }

    fn push(
        &mut self,
        kind: Option<RecordKind>,
        turn: Option<usize>,
        timestamp: Option<String>,
        usage: Option<Usage>,
    ) {
        if let Some(kind) = kind {
            self.records.push(Record {
                turn,
                timestamp,
                usage,
                kind,
            });
        }
    }

    pub(crate) fn finish(mut self) -> Vec<Record> {
        let turn = (self.turn > 0).then_some(self.turn);
        for (kind, timestamp, usage) in self.attached.drain(..) {
            self.records.push(Record {
                turn,
                timestamp,
                usage,
                kind,
            });
        }
        self.records
    }
}

/// Parse a Session file's text into a typed [`Session`] (ADR 0006). Pure and
/// infallible: unparseable or unrecognised lines contribute no Record, so one
/// malformed line never aborts the parse.
pub(crate) fn read(text: &str) -> Session {
    let mut meta = SessionMeta::default();
    let mut records = RecordBuilder::default();

    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        // Header metadata accumulates from whichever Records carry it.
        if let Some(ts) = value.get("timestamp").and_then(|t| t.as_str()) {
            // ISO 8601 sorts lexically, so the max string is the newest record.
            if meta.timestamp.as_deref().is_none_or(|cur| ts > cur) {
                meta.timestamp = Some(ts.to_string());
            }
        }
        if meta.branch.is_none() {
            if let Some(b) = value.get("gitBranch").and_then(|b| b.as_str()) {
                meta.branch = Some(b.to_string());
            }
        }
        // The cwd is constant within a Session, so the first one seen is enough.
        if meta.cwd.is_none() {
            if let Some(c) = value.get("cwd").and_then(|c| c.as_str()) {
                meta.cwd = Some(c.to_string());
            }
        }

        let ty = value.get("type").and_then(|t| t.as_str());
        // The turn number ticks on every Message Record, *before* its content is
        // inspected — a contentless Message still consumes a turn (ADR 0002).
        let kind = match ty {
            Some("user") => user_kind(&value),
            Some("assistant") => assistant_kind(&value),
            Some("ai-title") => value.get("aiTitle").and_then(|t| t.as_str()).map(|t| {
                meta.title = Some(t.to_string());
                RecordKind::Title(t.to_string())
            }),
            _ => None,
        };
        // Per-Record timestamp and Usage travel with the Record for the usage
        // breakdown. Only assistant Records carry Usage; every Record keeps its
        // own timestamp when the envelope has one.
        let timestamp = value
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(str::to_string);
        let usage = if matches!(ty, Some("assistant")) {
            claude_usage(&value)
        } else {
            None
        };

        if matches!(ty, Some("user") | Some("assistant")) {
            records.message(kind, timestamp, usage);
        } else {
            records.metadata(kind, timestamp, usage);
        }
    }

    Session {
        meta,
        records: records.finish(),
    }
}

/// Index every `tool_use` in the Records by its id, mapping to `(tool name,
/// key argument)` — the join a `tool_result` (and a Failure) needs to recover
/// the tool that produced it. Built lazily by the two projections that need it
/// (`show`, `--failed`), not on the search path.
pub(crate) fn tool_index(records: &[Record]) -> HashMap<String, (String, Option<String>)> {
    let mut index = HashMap::new();
    for record in records {
        let RecordKind::Assistant(blocks) = &record.kind else {
            continue;
        };
        for block in blocks {
            if let AssistantBlock::ToolUse {
                id: Some(id),
                name,
                input,
            } = block
            {
                index.insert(id.clone(), (name.clone(), tool_key_arg(input)));
            }
        }
    }
    index
}

/// Classify a `user` Record's content. A plain string is a Prompt; an array is
/// tool_result (and occasionally text) Blocks. Malformed content yields no
/// Record (the turn it consumed is already counted).
fn user_kind(value: &Value) -> Option<RecordKind> {
    match value.pointer("/message/content") {
        Some(c) if c.is_string() => Some(RecordKind::Prompt(c.as_str().unwrap().to_string())),
        Some(c) if c.is_array() => Some(RecordKind::UserBlocks(
            c.as_array()
                .unwrap()
                .iter()
                .filter_map(user_block)
                .collect(),
        )),
        _ => None,
    }
}

/// Parse one Block of a user Message's `content` array. Kept losslessly — both
/// `text` and `tool_result` — even though `search` only matches the latter.
fn user_block(block: &Value) -> Option<UserBlock> {
    match block.get("type").and_then(|t| t.as_str()) {
        Some("text") => block
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| UserBlock::Text(t.to_string())),
        Some("tool_result") => Some(UserBlock::ToolResult {
            is_error: block
                .get("is_error")
                .and_then(|e| e.as_bool())
                .unwrap_or(false),
            exit_code: None,
            tool_use_id: block
                .get("tool_use_id")
                .and_then(|i| i.as_str())
                .map(str::to_string),
            text: tool_result_text(block),
        }),
        _ => None,
    }
}

/// The text of a `tool_result` Block: its `content`, which may be a plain string
/// or an array of text blocks (joined). Empty when neither is present.
fn tool_result_text(block: &Value) -> String {
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

/// Classify an `assistant` Record's content into its Blocks, in order. A missing
/// or non-array content yields no Record (its turn is already counted).
fn assistant_kind(value: &Value) -> Option<RecordKind> {
    let blocks = value
        .pointer("/message/content")
        .and_then(|c| c.as_array())?;
    Some(RecordKind::Assistant(
        blocks.iter().filter_map(assistant_block).collect(),
    ))
}

/// Extract Claude Code Usage from an `assistant` envelope: the model from
/// `message.model`, the call id from `message.id`, and the four token counts
/// from `message.usage` when present. Always returns `Some` for an assistant
/// envelope, so a call without `message.usage` still carries its model and
/// call id for grouping and display, with `None` token counts rendering blank
/// numeric cells. A missing or non-u64 token field becomes `None` rather than
/// aborting the parse; a missing or non-object `usage` means all four are `None`.
fn claude_usage(value: &Value) -> Option<Usage> {
    let usage = value.pointer("/message/usage");
    let as_u64 = |key: &str| usage.and_then(|u| u.get(key)).and_then(Value::as_u64);
    Some(Usage {
        input: as_u64("input_tokens"),
        cache_create: as_u64("cache_creation_input_tokens"),
        cache_read: as_u64("cache_read_input_tokens"),
        output: as_u64("output_tokens"),
        model: value
            .pointer("/message/model")
            .and_then(|m| m.as_str())
            .map(str::to_string),
        call_id: value
            .pointer("/message/id")
            .and_then(|id| id.as_str())
            .map(str::to_string),
    })
}

/// Parse one Block of an assistant Message's `content` array. Kept losslessly,
/// including empty `text` / `thinking` (a signature-only thinking block) — the
/// projections, not the parser, decide whether to drop empties (ADR 0006).
fn assistant_block(block: &Value) -> Option<AssistantBlock> {
    match block.get("type").and_then(|t| t.as_str()) {
        Some("text") => block
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| AssistantBlock::Text(t.to_string())),
        Some("thinking") => block
            .get("thinking")
            .and_then(|t| t.as_str())
            .map(|t| AssistantBlock::Thinking(t.to_string())),
        Some("tool_use") => Some(AssistantBlock::ToolUse {
            id: block.get("id").and_then(|i| i.as_str()).map(str::to_string),
            name: block
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string(),
            input: block.get("input").cloned().unwrap_or(Value::Null),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_turns_by_message_type_including_a_contentless_record() {
        // ADR 0002 invariant: the turn number ticks on every user/assistant
        // Record — even a contentless one (signature-only thinking) — and skips
        // noise Records, so search and show agree on the coordinate.
        let text = [
            r#"{"type":"queue-operation","operation":"enqueue"}"#,
            r#"{"type":"user","message":{"role":"user","content":"alpha one"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"","signature":"s"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":"alpha two"}}"#,
        ]
        .join("\n");

        let session = read(&text);

        // queue-operation skipped; user=1, contentless assistant=2, user=3.
        let turns: Vec<Option<usize>> = session.records.iter().map(|r| r.turn).collect();
        assert_eq!(turns, vec![Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn parses_user_string_as_prompt_and_user_array_as_blocks() {
        let text = [
            r#"{"type":"user","message":{"role":"user","content":"a prompt"}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"boom"}]}}"#,
        ]
        .join("\n");

        let records = read(&text).records;

        assert_eq!(records[0].kind, RecordKind::Prompt("a prompt".into()));
        assert_eq!(
            records[1].kind,
            RecordKind::UserBlocks(vec![UserBlock::ToolResult {
                is_error: true,
                exit_code: None,
                tool_use_id: Some("t1".into()),
                text: "boom".into(),
            }])
        );
    }

    #[test]
    fn parses_assistant_blocks_in_order_preserving_kind() {
        let text = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"hello"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#;

        let records = read(text).records;

        let RecordKind::Assistant(blocks) = &records[0].kind else {
            panic!("expected an assistant Record");
        };
        assert_eq!(blocks[0], AssistantBlock::Thinking("hmm".into()));
        assert_eq!(blocks[1], AssistantBlock::Text("hello".into()));
        assert!(matches!(&blocks[2], AssistantBlock::ToolUse { name, .. } if name == "Bash"));
    }

    #[test]
    fn ai_title_becomes_a_titled_record_and_sets_meta_title() {
        let session = read(r#"{"type":"ai-title","aiTitle":"Borrow chat"}"#);

        assert_eq!(session.meta.title.as_deref(), Some("Borrow chat"));
        assert_eq!(
            session.records[0].kind,
            RecordKind::Title("Borrow chat".into())
        );
        assert_eq!(
            session.records[0].turn, None,
            "a Title is metadata, not a turn"
        );
    }

    #[test]
    fn meta_takes_newest_timestamp_first_branch_and_first_cwd() {
        let text = [
            r#"{"type":"user","message":{"role":"user","content":"x"},"timestamp":"2026-01-01T00:00:00.000Z","gitBranch":"main","cwd":"E:\\proj"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"y"}]},"timestamp":"2026-06-01T00:00:00.000Z","gitBranch":"feature","cwd":"E:\\other"}"#,
        ]
        .join("\n");

        let meta = read(&text).meta;

        assert_eq!(
            meta.timestamp.as_deref(),
            Some("2026-06-01T00:00:00.000Z"),
            "newest wins"
        );
        assert_eq!(meta.branch.as_deref(), Some("main"), "first branch wins");
        assert_eq!(meta.cwd.as_deref(), Some("E:\\proj"), "first cwd wins");
    }

    #[test]
    fn skips_unparseable_blank_and_noise_lines_without_a_record() {
        // Lenient parsing: a bad, blank, or unrecognised line contributes no
        // Record, so one malformed line never aborts the parse.
        let text = [
            "this is not json at all",
            "",
            r#"{"type":"queue-operation","operation":"enqueue"}"#,
            r#"{"type":"user","message":{"role":"user","content":"kept"}}"#,
        ]
        .join("\n");

        let session = read(&text);

        assert_eq!(session.records.len(), 1, "only the user Message survives");
        assert_eq!(session.records[0].kind, RecordKind::Prompt("kept".into()));
        assert_eq!(session.records[0].turn, Some(1));
    }

    #[test]
    fn tool_index_joins_tool_use_id_to_name_and_command() {
        let records = read(
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
        )
        .records;

        let index = tool_index(&records);

        assert_eq!(
            index.get("t1"),
            Some(&("Bash".to_string(), Some("cargo test".to_string())))
        );
    }
}
