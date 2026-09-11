---
name: agsearch
description: >-
  Search local Claude Code and Codex conversation history with the `agsearch` CLI.
  Use when the user asks to recall, find, or reopen a PAST session ("did we
  ever discuss X", "find the conversation where we set up Y", "what did I
  decide about Z last week"), to list past sessions or projects, or to analyse
  failed tool calls across their history ("what's been failing", "why do my
  tool calls keep erroring"). NOT for searching the current project's source
  files (use Grep/Glob for that).
---

# agsearch: search past coding conversations

`agsearch` searches local Claude Code and Codex transcripts. It searches both
Stores by default. Use the binary to find and render Sessions, then interpret
and summarise the output. Run `agsearch --help` for the full flag list.

If `agsearch --version` fails, the binary is not installed; tell the user to
`cargo install --path .` from a clone of this repo rather than guessing at
answers.

## The core workflow: session-id

Every verb is welded together by the **short session-id** (a git-style unique
prefix) printed at the start of each result header. Find → copy the id → open:

```console
$ agsearch "diesel migration"        # find — note the id in each header
$ agsearch show 4c28878f             # read — the whole transcript
$ agsearch show 4c28878f --around 220  # read — just the turns near turn 220
$ agsearch "panic" --session 4c28878f  # search within that one conversation
$ agsearch current --id-only          # full id of the Session that invoked this
```

Prefer `show` over reading raw `.jsonl` files — it renders the transcript for
you. Result trailers print the exact `show` command to run next.

## Verbs

- `agsearch "<terms>"` — search (the default verb; case-insensitive literal
  substring). `-e` for regex, `-s` for case-sensitive.
- `agsearch show <id>` — render a session as a transcript; `--around N`
  windows it. `<id>` accepts a short id-prefix, `current` for the top-level
  Current Session, or `current-thread` for the calling thread.
- `agsearch current` — print the Current Session (Harness, full id, Project,
  title, path, caller kind). `--id-only` prints the full top-level id;
  `--path` prints the source file. Fails outside a supported Harness, when
  the identity is missing from the configured Stores, or when Claude and Codex
  both identify a Session unless `--harness` is passed.
- `agsearch export <SESSION> <DEST>` — write one Session snapshot to one
  destination. `<SESSION>` accepts a short id-prefix, `current` (top-level
  only, never a family bundle), or `current-thread`. Markdown is the default:
  a readable document with provenance (title, full id, Harness, Project,
  source timestamp, export timestamp, snapshot status) plus the Transcript.
  `--format raw` copies the Harness Session file exactly, with no added
  metadata and no prompt. `-` writes to stdout. Every Export is a
  point-in-time snapshot that finishes without waiting; Markdown ignores an
  incomplete trailing record and marks itself as a snapshot. An existing file
  is rejected unless `--force` is passed.
- `agsearch sessions` / `agsearch projects` — list sessions in scope /
  projects in the store, newest first. Use these to orient before searching.
- `agsearch --failed [query]` — list failed tool calls by structure; the
  query becomes an optional filter on command/error. `--full` prints whole
  error texts; `--stats` aggregates into a counts table by tool and error
  signature.
- `agsearch --file <SELECTOR>` — list file Touches by File Selector instead of
  searching text. `--written` keeps only write Touches. See `agsearch --help` for selector rules.

Search, `--failed`, `--stats`, and `--file` cover **past** Sessions. They exclude the
Current Session Family, which contains the Current Session and its subagent
threads. This prevents a search from returning the Prompt that started the
search.

Add `--include-current` when the user wants to include the current work. Use
`--session current` to search the top-level Current Session directly, or
`--session current-thread` to search the calling thread. An explicit
Session id selects that Session even when it belongs to the family. `sessions`
and `projects` still include the Current Session.

## Scope

With no flag, every verb covers the **current working directory's** project —
when the user asks about a different project, reach for a scope flag:

| User intent | Flag |
| --- | --- |
| "anywhere" / "any project" | `--all` |
| names another project | `--project <name-substr>` |
| one known conversation | `--session <id-prefix\|current\|current-thread>` |
| "this Session too" or "including now" | `--include-current` |
| "last week" / "since May" | `--since 1w` / `--since 2026-05-01` |

`--since` composes with every scope; `--all` and `--project` conflict.

## Gotchas `--help` won't tell you

- Exit `0` **even with no matches**; `1` on invalid regex, an unresolvable
  config dir, an unresolvable Session selector, unavailable or ambiguous
  current context, or an existing Export destination without `--force`. Empty
  output means "searched fine, found nothing" — say that rather than retrying
  blindly.
- Default search covers user/assistant text and titles. When a plain search
  comes up empty, retry with `--thinking`, `--tools`, or `--all-content` — the
  topic may live in a reasoning block or a tool call.
- `--tools` matches tool input serialised as raw JSON, so structural tokens
  match noisily — a fallback, not a first pass.
- Windows and WSL keep separate histories; `--claude-dir` points at the other
  Claude Store and `--codex-dir` points at the other Codex Store.
- Use `--harness claude` or `--harness codex` only when the user names a
  Harness, or when `agsearch current` reports that both Harnesses identified a
  Session. The default cross-Store search is better when the Harness is unknown.
- Codex subagent Sessions are excluded by default. Use `--include-subagents`
  when the user asks about worker activity. The output marks those Sessions.
- `--include-subagents` still hides the Current Session's workers because
  historical search excludes the whole Current Session Family. Pass
  `--include-current` to include them.
- A search for a recent Prompt in *this* Session finds nothing by default. This
  is the Current Session Family exclusion. Add `--include-current`, or read the
  Current Session with
  `agsearch show $(agsearch current --id-only)`.
- Output is human-readable only (no JSON mode); read it directly.
- Too many capped sessions? Raise `-m` (default 3 matches shown per session,
  `0` = unlimited) or narrow the terms.

## Reporting back

Summarise in plain language and point the user at the session: project, title,
date, and the `agsearch show <id>` command to reopen it.
