---
name: agsearch
description: >-
  Search your local Claude Code conversation history with the `agsearch` CLI.
  Use when the user asks to recall, find, or reopen a PAST session ("did we
  ever discuss X", "find the conversation where we set up Y", "what did I
  decide about Z last week"), to list past sessions or projects, or to analyse
  failed tool calls across their history ("what's been failing", "why do my
  tool calls keep erroring"). NOT for searching the current project's source
  files (use Grep/Glob for that).
---

# agsearch: search past Claude Code conversations

`agsearch` is a Rust CLI that searches the user's local Claude Code transcripts
(the `.jsonl` files under `~/.claude/projects/`). This skill is thin glue: the
binary does the mechanical work, you interpret the results, drill in, and
summarise. Run `agsearch --help` for the full flag surface — this file only
records what `--help` cannot tell you.

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
```

Prefer `show` over reading raw `.jsonl` files — it renders the transcript for
you. Result trailers print the exact `show` command to run next.

## Verbs

- `agsearch "<terms>"` — search (the default verb; case-insensitive literal
  substring). `-e` for regex, `-s` for case-sensitive.
- `agsearch show <id>` — render a session as a transcript; `--around N`
  windows it.
- `agsearch sessions` / `agsearch projects` — list sessions in scope /
  projects in the store, newest first. Use these to orient before searching.
- `agsearch --failed [query]` — list failed tool calls by structure; the
  query becomes an optional filter on command/error. `--full` prints whole
  error texts; `--stats` aggregates into a counts table by tool and error
  signature.

## Scope

With no flag, every verb covers the **current working directory's** project —
when the user asks about a different project, reach for a scope flag:

| User intent | Flag |
| --- | --- |
| "anywhere" / "any project" | `--all` |
| names another project | `--project <name-substr>` |
| one known conversation | `--session <id-prefix>` |
| "last week" / "since May" | `--since 1w` / `--since 2026-05-01` |

`--since` composes with every scope; `--all` and `--project` conflict.

## Gotchas `--help` won't tell you

- Exit `0` **even with no matches**; `1` only on invalid regex or an
  unresolvable config dir. Empty output means "searched fine, found nothing" —
  say that rather than retrying blindly.
- Default search covers user/assistant text and titles. When a plain search
  comes up empty, retry with `--thinking`, `--tools`, or `--all-content` — the
  topic may live in a reasoning block or a tool call.
- `--tools` matches tool input serialised as raw JSON, so structural tokens
  match noisily — a fallback, not a first pass.
- Windows and WSL keep separate histories; `--claude-dir` points at the other
  one.
- Output is human-readable only (no JSON mode); read it directly.
- Too many capped sessions? Raise `-m` (default 3 matches shown per session,
  `0` = unlimited) or narrow the terms.

## Reporting back

Summarise in plain language and point the user at the session: project, title,
date, and the `agsearch show <id>` command to reopen it.
