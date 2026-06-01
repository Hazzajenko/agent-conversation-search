---
name: ccsearch
description: >-
  Recall or find a PAST Claude Code conversation by searching your local
  conversation history with the `ccsearch` CLI. Use when the user asks to
  recall, find, or look up an earlier session, e.g. "did we ever discuss X",
  "find the conversation where we set up Y", "what did I decide about Z",
  "which session was that in", "have we talked about this before". NOT for searching
  the current project's source files (use Grep/Glob for that).
---

# ccsearch: recall a past Claude Code conversation

`ccsearch` is a fast Rust CLI that searches the user's local Claude Code
conversation history (the `.jsonl` transcripts under `~/.claude/projects/`).
This skill is thin glue: the binary does the mechanical search, you interpret
the results, drill in for full context, and summarise.

## When to Use

Trigger on requests to recall or locate a **previous conversation**:

- "Did we ever discuss the borrow checker issue?"
- "Find the session where we set up the CI pipeline."
- "What did I decide about the auth flow last week?"
- "Which conversation was that bug in?"
- "Have we hit this error before?"

**Do not** use this for searching the current project's code or files; that is
what `Grep`/`Glob` are for. This skill searches conversation transcripts, not
the working tree.

## Prerequisite: the binary must be on PATH

This skill calls the `ccsearch` executable. Confirm it is installed:

```bash
ccsearch --version
```

If that fails, the binary is not installed. Install it from the
`claude-code-conversation-search` repo:

```bash
cargo install --path .          # from a local clone of the repo
# or
cargo install --git <repo-url>  # if published to a git remote
```

Until the binary is on PATH, this skill cannot run; tell the user to install
it rather than guessing at answers.

## How to search

`ccsearch` matches a **case-insensitive literal substring** by default and
prints human-readable output (there is no JSON mode; the output is meant to be
read directly by you).

Pick the scope from what the user asked for:

| User intent | Command |
| --- | --- |
| "in this project" (the default) | `ccsearch "<terms>"` |
| "anywhere" / "any project" / "everywhere" | `ccsearch --all "<terms>"` |
| names another project | `ccsearch --project "<name-substr>" "<terms>"` |

`--all` searches every project's history; `--project <SUBSTR>` narrows to
projects whose directory name contains `SUBSTR` (case-insensitive). `--all` and
`--project` are mutually exclusive.

> Scope note: with no scope flag, `ccsearch` searches the history of the
> **current working directory's** project. When invoked from a different
> project than the one the user is asking about, reach for `--all` or
> `--project`.

### Reading the output and drilling in

1. Run the search and read the printed sessions. Each result is a session
   header followed by up to N indented `role: snippet` lines centred on the
   match. Sessions are sorted newest-first.
2. The snippet is often enough to answer "did we discuss X". When you need the
   **full** conversation, get the file paths and read around the match:

   ```bash
   ccsearch --all -l "<terms>"   # -l / --files prints only matching .jsonl paths
   ```

   Then `Read` the relevant `.jsonl` to recover surrounding context (each line
   is one JSON record: `user` / `assistant` / `tool_use` / `tool_result`).
3. Summarise what you found in plain language and **point the user at the
   session**: name the project, the session title/date, and what was decided.

### Output shape (example)

```
e--projects-myapp · Wiring up the auth flow · 2026-05-20 · main
  user: …should we use the borrow checker trick here or just clone the…
  assistant: …the borrow checker will reject that because the mutable borrow…
  … +2 more matches
```

A header is `project · title · date · branch` (date/branch omitted when
absent). `  … +N more matches` appears when a session is capped by
`--max-per-session`.

## Flag reference

The complete flag surface (verified against the binary):

| Flag | Effect |
| --- | --- |
| `<QUERY>` (positional, required) | text to search for (literal substring by default) |
| `--all` | search every project (conflicts with `--project`) |
| `--project <SUBSTR>` | projects whose dir name contains SUBSTR (case-insensitive) |
| `--regex`, `-e` | treat the query as a regular expression |
| `--case-sensitive`, `-s` | case-sensitive matching (default ignores case) |
| `--thinking` | also search assistant thinking blocks |
| `--tools` | also search tool calls (`tool_use`) and tool results (`tool_result`) |
| `--all-content` | equivalent to `--thinking --tools` |
| `--max-per-session <N>`, `-m` | cap matches shown per session (default 3; `0` = unlimited) |
| `--files`, `-l` | print only matching `.jsonl` paths (grep `-l` style) |
| `--claude-dir <PATH>` | override the Claude config dir (else `$CLAUDE_CONFIG_DIR`, else `~/.claude`) |

Exit codes: `0` on success **even when there are no matches**; `1` only on an
invalid regex, an unresolvable config dir, or an unreadable working directory.
A `0` exit with empty output means "searched fine, found nothing", so say that
rather than retrying blindly.

## Tips

- Default search covers user and assistant message text plus titles. Add
  `--thinking` to include reasoning blocks, `--tools` for tool calls/results,
  or `--all-content` for everything. Reach for these when a plain search comes
  up empty but you suspect the topic lived in a tool call or thinking block.
- `--tools` serialises tool input as raw JSON, so it can match structural
  tokens noisily. Prefer it as a fallback, not a first pass.
- Use `-e`/`--regex` for patterns (e.g. `ccsearch -e "fn \w+_test" --all`).
- If a query returns too many capped sessions, raise `-m` or narrow the terms.
