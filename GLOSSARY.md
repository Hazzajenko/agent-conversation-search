# Agent Conversation Search (agsearch)

A CLI tool that searches the local conversation history coding harnesses write to disk. The domain is the on-disk transcript formats and the act of searching them, not the harness products themselves.

## Language

**Harness**:
A coding tool that writes conversations to disk: Claude Code (`claude`), Codex (`codex`), or OpenCode (`opencode`). Each Harness has its own Store layout and Record format. A machine may have any subset of Harnesses installed. Selected with `--harness`.
_Avoid_: agent (collides with subagent threads), tool, source, model.

**Project**:
A working directory in which a Harness was used. It is a logical concept shared across Harnesses, so the same repo used with Claude Code and Codex is one Project. Derived per Harness: for Claude Code, from the directory name under `~/.claude/projects/`, with path separators replaced by `-` and case-variant directories folded together (see ADR 0001). For Codex, it comes from the `cwd` recorded in the Session (see ADR 0009). For OpenCode, it comes from the Session's `directory`. OpenCode's own project id is not used.
_Avoid_: folder, workspace, repo.

**Session**:
A single conversation. Claude Code and Codex store it as exactly one `.jsonl` file. OpenCode stores it as one `session` row in its database. Identified by a `sessionId`, looked up transparently across all Stores. Codex subagent threads (rollout files whose meta marks them as spawned workers) are not Sessions by default. They are excluded from listing and search unless opted in. A forked Codex thread is its own Session. An OpenCode child session, which has a parent session, is a subagent thread. An archived OpenCode session is still a Session.
_Avoid_: chat, thread. (User-facing text may say "conversation" as a synonym for Session.)

**Current Session**:
The top-level Session from which a Harness invokes `agsearch`. When a spawned worker invokes it, the Current Session remains the top-level conversation. The worker's own subagent thread is selected explicitly when needed.
_Avoid_: active chat, current conversation.

**Current Session Family**:
The Current Session together with every subagent thread descended from it. Multi-Session analysis excludes this entire family by default, including when subagent Sessions are otherwise included.
_Avoid_: current group, active sessions.

**Record**:
One unit of a Session in the Harness's own format. For Claude Code and Codex it is one line of the Session file, a single JSON object. Claude Code uses typed lines like `user` / `assistant` / `ai-title`. Codex uses `{timestamp, type, payload}` envelopes such as `session_meta` and `response_item`. For OpenCode it is one `message` row together with its parts. Parts of kind `text`, `reasoning`, and `tool` become Blocks. Other parts, such as `step-start` and `snapshot`, are noise. Every line is a Record, but not every Record is a Message.
_Avoid_: line, entry, event.

**Message**:
A Record of type `user` or `assistant`, which is an actual turn in the conversation. A user Message's content is normally a plain string. An assistant Message's content is an array of blocks (`text`, `thinking`, `tool_use`).
_Avoid_: turn, post.

**Block**:
One element of a Message's `content` array. A user Message's Blocks are
`tool_result`, and occasionally `text`. An assistant Message's Blocks are
`text`, `thinking`, or `tool_use`. Parsed into a typed value with **distinct
kinds for user and assistant**, so a user Block can never be a `tool_use` nor an
assistant Block a `tool_result` (see ADR 0006).
_Avoid_: element, item, part.

**Prompt**:
The text content of a user Message. It is what the person typed to Claude. Distinct from a Query.
_Avoid_: input, request.

**Reply**:
The text content of an assistant `text` block. It is what Claude said back. Excludes thinking and tool blocks.
_Avoid_: response, answer, completion.

**Title**:
The one-line AI-generated summary of a Session. Derived per Harness: Claude Code carries it in an `ai-title` Record inside the Session file. Codex keeps it as `thread_name` in the central `session_index.jsonl`. OpenCode keeps it as the session's `title`. A Session with no entry is untitled. An OpenCode placeholder title such as "New session - 2026-09-26T..." also counts as untitled.
_Avoid_: name, subject, summary.

**Query**:
The search string a user passes to *this tool*. Never confuse with Prompt. A Prompt is data being searched, and a Query is the thing searching it.
_Avoid_: search term (in code), prompt.

**Store**:
The root directory where one Harness keeps its Sessions: `~/.claude/projects/` for Claude Code (`--claude-dir` / `$CLAUDE_CONFIG_DIR`), `~/.codex/sessions/` for Codex (`--codex-dir` / `$CODEX_HOME`), `~/.local/share/opencode/` for OpenCode (`--opencode-dir` / `$XDG_DATA_HOME`). The OpenCode Store holds one database, `opencode.db`, for all Sessions. The older OpenCode JSON files beside it are not read. A machine has one Store per installed Harness. Searches span all Stores that exist, and a missing Store is silently skipped. Windows and WSL have separate Stores.
_Avoid_: database, index, cache.

**Match**:
A single occurrence of the Query inside one Record. A Session can contain many Matches, and results are grouped by Session.
_Avoid_: hit, result.

**Snippet**:
The truncated, Query-centered excerpt of a Record's text shown for a Match, with the matched span highlighted.
_Avoid_: excerpt, preview, context.

**Transcript**:
The full, human-readable rendering of a single Session in turn order, produced by the `show` verb. Query-agnostic. Unlike a Snippet, it is not centered on a Match. Shows Messages only and drops the noise Records. Tool calls render as compact one-liners and failed tool results are flagged.
_Avoid_: dump, log, printout, history.

**Export**:
A point-in-time copy of a Session written outside its Store. An Export is either a portable, human-readable document containing the Transcript and its provenance, or an exact copy of the Harness's raw Session data. For Claude Code and Codex the raw data is the Session file. For OpenCode it is a JSON document with the session row, then each message with its parts, in turn order.
_Avoid_: backup, dump.

**Failure**:
A tool call that errored. Detected per Harness: Claude Code marks it structurally (`tool_result` with `is_error: true`, joined via `tool_use_id` to the `tool_use` that names the tool and command). Codex has no such flag, so failure is inferred from the tool-output payload. OpenCode marks it structurally as a `tool` part with status `error`. An OpenCode shell command that exits non-zero is not a Failure unless its status is `error`. The unit that failure-analysis lists and counts. Distinct from a Match, because a Failure is found by structure, not by a Query.
_Avoid_: error, crash, exception, bug.

**Touch**:
A tool call in a Session that reads or writes a specific file. Has a kind: `read` (Read, OpenCode `read`) or `write` (Edit, Write, MultiEdit, NotebookEdit, Codex `apply_patch`, OpenCode `edit`, `write`, and `patch`). An OpenCode `patch` part, which records a snapshot diff, is not a Touch. Found by structure, like a Failure, not by a Query. A file injected into the conversation by the Harness (for example `CLAUDE.md` in a system reminder) is not a Touch. Shell commands that open a file are not a Touch.
_Avoid_: access, hit, reference, usage.

**File Selector**:
The path fragment a user passes to select Touches. Matches when its segments equal the trailing segments of the Touch's path, case-insensitive, with `/` and `\` treated as equal. A trailing separator is ignored (`docs/` ≡ `docs`). A selector that is empty after trimming and stripping is rejected. `CLAUDE.md` matches every `CLAUDE.md` in every Project. `docs/adr/0001-x.md` matches only that trailing path.
_Avoid_: file filter, glob, pattern, file.

**Usage**:
The token counts a Harness records for one model call: input, output, cache-write and cache-read tokens, plus the model name. Claude Code attaches it to each assistant Message. Codex records it as a `token_count` Record per turn. OpenCode attaches it to each assistant Message, and its reasoning tokens count as output. A Session's Usage is the sum over its calls. A family's Usage is the sum over the parent Session and its subagent threads. Tokens only. No money is involved.
_Avoid_: cost, spend, bill, price, tokens (as a term).

## Example dialogue

> **Dev:** When I run a search, does a hit mean the Query matched a whole Session?
> **Domain expert:** No. The Query matches inside a single Record. A Session can have many matching Records. We group hits by Session so you see which conversation to reopen.
> **Dev:** And if the Query appears in Claude's thinking?
> **Domain expert:** Thinking isn't a Reply, so by default it's not searched. A Reply is only the `text` blocks. Thinking is opt-in.
