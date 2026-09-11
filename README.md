# agsearch

Search your local coding conversation history from the command line.

Claude Code and Codex record conversations as `.jsonl` transcripts. `agsearch`
searches both Stores by default, so you do not need to remember which Harness
you used.

```console
$ agsearch "borrow checker"
claude · d712581e · E:\projects\rust\demo · Lifetimes and the borrow checker · 2026-06-02 · main
  [14] user: how do I satisfy the borrow checker here without cloning
  [16] assistant: …the borrow checker is complaining because the reference outlives…
  … +3 more  ›  agsearch show d712581e

$ agsearch show d712581e        # reopen the whole conversation
$ agsearch current             # inspect the Session that invoked this command
```

## Why

The transcripts use Harness-specific layouts and JSON formats. `agsearch`
turns them into five jobs:

- **find** a conversation — `agsearch <query>`
- **read** a conversation — `agsearch show <id>`
- **inspect** the Current Session — `agsearch current`
- **preserve** a conversation — `agsearch export <id> <file>`
- **analyse** what went wrong — `agsearch --failed` / `--stats`

## Install

Requires a [Rust toolchain](https://rustup.rs/).

```console
# from a clone of this repo
cargo install --path .          # installs the `agsearch` binary onto your PATH
# or just build it
cargo build --release           # ./target/release/agsearch
```

`agsearch` reads `$CLAUDE_CONFIG_DIR` or `~/.claude` for Claude Code and
`$CODEX_HOME` or `~/.codex` for Codex. Use `--claude-dir <PATH>` or
`--codex-dir <PATH>` to override a Store. A missing Store is skipped.

Windows and WSL keep separate histories. Run `agsearch` in the environment that
has the Sessions, or point the Store flags at the other environment.

## The core workflow

Every verb is welded together by one identifier: the **short session-id** (a
git-style unique prefix). `search` and `--failed` print it; `show` resolves it.
That is the whole loop — find something, copy its id, open it:

```console
$ agsearch "diesel migration"      # find — note the id in each header
$ agsearch show 4c28878f           # read — the full transcript
$ agsearch show 4c28878f --around 220   # read — just the turns around turn 220
```

You can also pipe by path instead of copying ids:

```console
$ agsearch -l "diesel migration" | agsearch show -   # open the first match
```

## Verbs

### `search` (the default)

`agsearch <query>` searches the **current directory's** Project for a
case-insensitive substring, grouped by Session, newest first. It is the default
verb, so the word `search` is optional (`agsearch foo` ≡ `agsearch search foo`).

| Flag | Effect |
|------|--------|
| `-e`, `--regex` | treat the query as a regular expression |
| `-s`, `--case-sensitive` | match case-sensitively |
| `--thinking` | also search assistant thinking blocks |
| `--tools` | also search tool calls and tool results |
| `--all-content` | search everything (`--thinking --tools`) |
| `-m`, `--max-per-session <N>` | cap matches shown per Session (`0` = unlimited; default 3) |
| `-l`, `--files` | print only matching file paths, for piping |
| `--file <SELECTOR>` | list file Touches by File Selector instead of searching text |
| `--harness <claude\|codex>` | search only one Harness |
| `--include-subagents` | include Codex subagent Sessions and mark them in output |
| `--include-current` | include the Current Session Family (excluded by default) |

By default only Prompts, Replies, and Titles are searched — thinking and tool
content are opt-in.

Search covers **historical** conversations. When a Harness identifies the
Current Session, search excludes its Current Session Family. This prevents a
search from matching the Prompt that started the search.

Pass `--include-current` to include the family. Pass `--session current` to
search the top-level Current Session directly, or `--session current-thread`
to search the calling thread (the same Session at the top level, the worker's
own thread when called from a spawned worker). A Session id selects that
Session even when it belongs to the family.

The exclusion also covers `--failed` and `--stats`. When
`--include-subagents` is active, it hides every worker in the family. Outside a
supported Harness, no Current Session is available and search excludes
nothing.

When identity resolves only partly, search hides every family it can identify.
If both Harnesses identify a Session, search excludes both families. If a
family ancestor is missing from the Store, search still excludes the calling
thread and its workers.

### `show` — read a whole conversation

`agsearch show <id>` renders a Session as a readable transcript (tool calls
collapse to one-liners; failed tool calls are flagged; thinking is hidden).

| Flag | Effect |
|------|--------|
| `--thinking` | expand assistant thinking blocks |
| `--around <turn>` | show only the turns around this turn (e.g. a search hit) |
| `--context <N>` | turns of context on either side of `--around` (default 3) |

`<id>` is a git-style prefix resolved across your whole history; `current`
renders the top-level Current Session and `current-thread` renders the calling
thread (the same Session at the top level); `-` reads a file path from stdin.

### `current` — inspect the invoking Session

`agsearch current` prints the Current Session identified by the Harness that
launched the command: Harness, full Session ID, Project, title, source path,
and whether the caller is a worker. It does not guess from the newest
conversation.

| Flag | Effect |
|------|--------|
| `--id-only` | print only the full top-level Session ID |
| `--path` | print only the source Session path |

Claude identity comes from `$CLAUDE_CODE_SESSION_ID`. Codex identity comes from
`$CODEX_SESSION_ID` (the top-level Session) and `$CODEX_THREAD_ID` (the
calling thread). At the top level those two identify the same Session; a
spawned worker still resolves `current` to the top-level Session.

If no supported Harness identity is available, the command fails rather than
picking a Session. If the identity is not in the configured Stores, it fails.
If both Harnesses supply a valid identity, it reports the ambiguity until you
pass `--harness claude` or `--harness codex`.

```console
$ agsearch current
Harness: claude
Session: 11111111-aaaa-bbbb-cccc-ddddeeee0001
Project: E:\projects\demo
Title: Lifetimes and the borrow checker
Path: C:\Users\you\.claude\projects\E--projects-demo\11111111-aaaa-bbbb-cccc-ddddeeee0001.jsonl
Caller: top-level

$ agsearch current --id-only
11111111-aaaa-bbbb-cccc-ddddeeee0001
```

### `export` — preserve a Session snapshot

`agsearch export <SESSION> <DEST>` writes one point-in-time snapshot of one
Session to one destination. The Session accepts a unique id prefix, `current`
for the top-level Current Session, or `current-thread` for the calling thread.
`current` exports only the top-level Session, never a combined family document.
The destination is required; `-` writes the Export to standard output.

| Flag | Effect |
|------|--------|
| `--format markdown\|raw` | readable document (the default) or an exact raw copy |
| `--force` | overwrite the destination when it already exists |
| `--thinking` | expand assistant thinking blocks in Markdown Export |

Readable Export (Markdown) is a portable document with provenance followed by
the Transcript. Provenance carries the title, full Session ID, Harness,
Project, source timestamp, export timestamp, and snapshot status, so the file
identifies its origin without `agsearch`. The Transcript body uses the same
renderer as `show` — Messages only, tool calls as compact one-liners, failed
tool results flagged, thinking hidden unless `--thinking` is passed.

Raw Export (`--format raw`) copies the Harness Session file exactly, with no
added metadata and no interactive confirmation beyond the explicit format
option. It is the way to preserve the source data.

Every Export is a snapshot: it captures the content available when the command
runs and finishes without waiting for the Harness. A Markdown snapshot renders
every complete Record and ignores an incomplete trailing JSONL record; the
document identifies itself as a snapshot so nobody mistakes it for a final
transcript. A raw snapshot contains the source bytes captured by the operation.

An existing destination file is rejected unless `--force` is passed, so an
earlier snapshot is never destroyed silently.

```console
$ agsearch export 4c28878f transcript.md        # readable snapshot
$ agsearch export current transcript.md         # preserve the Current Session
$ agsearch export current-thread worker.md      # preserve only the calling thread
$ agsearch export 4c28878f - | less             # pipe through stdout
$ agsearch export 4c28878f raw.jsonl --format raw --force
```

### `sessions` — list conversations

`agsearch sessions` lists the Sessions in scope, newest first, one per line —
for when you want to reopen a recent conversation but don't remember anything to
search for. Each row is paste-able into `show`.

```console
$ agsearch sessions
codex · de151981 · E:\projects\rust\demo · Test app runtime and functionality · 2026-06-02
claude · d712581e · E:\projects\rust\demo · Lifetimes and the borrow checker · 2026-06-02 · main
```

### `projects` — list projects

`agsearch projects` lists every Project in your history (whole-Store), newest
first, with how many Sessions each holds and when it was last touched. The name
is the real path, recovered from the transcripts.

```console
$ agsearch projects
E:\projects\rust\agent-conversation-search · 7 sessions · 2026-06-02
E:\projects\games\creature-game · 43 sessions · 2026-06-01
```

## Failure analysis

Find and aggregate **failed tool calls** by structure, independent of any
Query. Failure analysis covers historical conversations. It excludes the
Current Session Family unless you pass `--include-current`.

```console
$ agsearch --failed                # list failed tool calls, joined to the command
$ agsearch --failed cargo          # filter failures by command / error text
$ agsearch --stats                 # aggregate into a counts table by tool + error
```

```console
$ agsearch --stats
19  ✗ PowerShell  error:
 3  ✗ Edit        String to replace not found
… +2 more singleton signatures
```

One-off signatures fold into the trailing `+N more` line so the table stays
readable at `--all` scale (unless every row is a one-off, in which case they
are shown). Use `--failed` to see every failure individually.

Add `--full` to a `--failed` listing to print each failure's complete error text
instead of the single salient line.

## Scope and filters

These compose with `search`, `--failed`, `--stats`, and (where noted) the
listing verbs:

| Flag | Effect | Applies to |
|------|--------|-----------|
| _(default)_ | the current directory's Project | search, sessions |
| `--all` | every Project in your history | search, sessions |
| `--project <substr>` | Projects whose name contains the substring | search, sessions, projects |
| `--session <selector>` | one Session, selected by id-prefix, `current`, or `current-thread`. The selected Session remains included when it belongs to the Current Session Family | search |
| `--file <SELECTOR>` | list Touches of the selected file (File Selector) | search |
| `--include-current` | put the Current Session Family back into the results | search, `--failed`, `--stats` |
| `--since <when>` | only Sessions/Projects touched since a duration (`3d`, `2w`, `1h`) or ISO date (`2026-05-01`) | all |

The default scope merges Sessions from both Harnesses by their encoded working
directory. `projects` shows one row when both Harnesses used the same directory.
Codex subagent Sessions stay excluded unless you pass `--include-subagents`.
`sessions` and `projects` are inventory verbs. They keep listing and counting
the Current Session.

## Output and exit codes

- Output is colorized on a TTY and plain when piped or under `NO_COLOR`.
- "No matches" / "No sessions" / "No projects" exit **0** — an empty result is
  not an error.
- Bad input (unparseable regex, unparseable `--since`, an ambiguous id, a
  missing Session, unavailable or ambiguous current context, an existing Export
  destination without `--force`) exits **non-zero** with a readable message.

## As a Claude Code plugin

This repo also ships a Claude Code skill (`skills/agsearch`, declared in
`.claude-plugin/plugin.json`) that lets Claude recall past conversations for you
— "did we ever discuss X?", "find the session where we set up Y". The skill is
thin glue over the same binary, so install `agsearch` on your PATH first.

## Concepts

The precise vocabulary this tool is built around — **Harness, Project, Session,
Record, Message, Query, Match, Failure, Store** — lives in [`CONTEXT.md`](CONTEXT.md).
The reasoning behind the bigger design decisions is in
[`docs/adr/`](docs/adr/) (locating Projects, the verb/handoff model, failure
grouping, the listing verbs).

## License

MIT.
