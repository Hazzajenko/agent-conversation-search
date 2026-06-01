# Claude Code Conversation Search

A CLI tool that searches the local conversation history Claude Code writes to disk. The domain is the on-disk transcript format and the act of searching it — not the Claude Code product itself.

## Language

**Project**:
A working directory in which Claude Code was used. On disk it is one directory under `~/.claude/projects/`, named by replacing every path separator in the working directory with `-`.
_Avoid_: folder, workspace, repo.

**Session**:
A single conversation, stored as exactly one `.jsonl` file inside a Project directory. Identified by a `sessionId`.
_Avoid_: chat, thread. (User-facing surfaces may say "conversation" as a synonym for Session.)

**Record**:
One line of a Session file — a single JSON object. Every line is a Record, but not every Record is a Message (e.g. `queue-operation`, `mode`, `attachment`).
_Avoid_: line, entry, event.

**Message**:
A Record of type `user` or `assistant` — an actual turn in the conversation. A user Message's content is normally a plain string; an assistant Message's content is an array of blocks (`text`, `thinking`, `tool_use`).
_Avoid_: turn, post.

**Prompt**:
The text content of a user Message — what the person typed to Claude. Distinct from a Query.
_Avoid_: input, request.

**Reply**:
The text content of an assistant `text` block — what Claude said back. Excludes thinking and tool blocks.
_Avoid_: response, answer, completion.

**Title**:
The one-line AI-generated summary of a Session, carried in an `ai-title` Record.
_Avoid_: name, subject, summary.

**Query**:
The search string a user passes to *this tool*. Never confuse with Prompt — a Prompt is data being searched, a Query is the thing searching it.
_Avoid_: search term (in code), prompt.

**Store**:
The root directory that holds all Projects (`~/.claude/projects/`, or wherever `--claude-dir` / `$CLAUDE_CONFIG_DIR` points). One machine/environment has one active Store; Windows and WSL have separate Stores.
_Avoid_: database, index, cache.

**Match**:
A single occurrence of the Query inside one Record. A Session can contain many Matches; results are grouped by Session.
_Avoid_: hit, result.

**Snippet**:
The truncated, Query-centered excerpt of a Record's text shown for a Match, with the matched span highlighted.
_Avoid_: excerpt, preview, context.

**Transcript**:
The full, human-readable rendering of a single Session in turn order, produced by the `show` verb. Query-agnostic — unlike a Snippet, it is not centered on a Match. Shows Messages only (the noise Records are dropped); tool calls render as compact one-liners and failed tool results are flagged.
_Avoid_: dump, log, printout, history.

**Failure**:
A tool call that errored — a `tool_result` Record carrying `is_error: true`, joined via its `tool_use_id` back to the `tool_use` block that triggered it (which supplies the tool name and command). The unit that failure-analysis lists and counts; distinct from a Match (a Failure is found by structure, not by a Query).
_Avoid_: error, crash, exception, bug.

## Example dialogue

> **Dev:** When I run a search, does a hit mean the Query matched a whole Session?
> **Domain expert:** No — the Query matches inside a single Record. A Session can have many matching Records. We group hits by Session so you see which conversation to reopen.
> **Dev:** And if the Query appears in Claude's thinking?
> **Domain expert:** Thinking isn't a Reply, so by default it's not searched. A Reply is only the `text` blocks. Thinking is opt-in.
