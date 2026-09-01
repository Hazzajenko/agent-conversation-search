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
turns them into four jobs:

- **find** a conversation — `agsearch <query>`
- **read** a conversation — `agsearch show <id>`
- **inspect** the Current Session — `agsearch current`
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
| `--harness <claude\|codex>` | search only one Harness |
| `--include-subagents` | include Codex subagent Sessions and mark them in output |
| `--include-current` | include the Current Session Family (excluded by default) |

By default only Prompts, Replies, and Titles are searched — thinking and tool
content are opt-in.

Search looks at **historical** conversations: when a Harness identifies the
Current Session, that Session and every subagent thread descended from it — the
Current Session Family — are left out, so asking "did we discuss X?" cannot
match the prompt that asked. `--include-current` puts the family back, and
`--session <id>` searches a Session you name even when it is the current one.
The exclusion covers `--failed` and `--stats` too, and applies to family
workers when `--include-subagents` is on. Outside a supported Harness nothing
is excluded, because there is no Current Session to find. Where identity is
only partly resolvable, analysis errs towards hiding: if both Harnesses
identify a Session, both families go; if the Current Session itself is missing
from the Store, the calling thread and its workers still go.

### `show` — read a whole conversation

`agsearch show <id>` renders a Session as a readable transcript (tool calls
collapse to one-liners; failed tool calls are flagged; thinking is hidden).

| Flag | Effect |
|------|--------|
| `--thinking` | expand assistant thinking blocks |
| `--around <turn>` | show only the turns around this turn (e.g. a search hit) |
| `--context <N>` | turns of context on either side of `--around` (default 3) |

`<id>` is a git-style prefix resolved across your whole history; `-` reads a
file path from stdin.

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

Find and aggregate **failed tool calls** by structure — independent of any
query — to see what's been breaking. Like text search, this covers historical
conversations: the Current Session Family is excluded unless you pass
`--include-current`.

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
| `--session <prefix>` | a single Session, by id-prefix (includes it even when it is the Current Session) | search |
| `--include-current` | put the Current Session Family back into the results | search, `--failed`, `--stats` |
| `--since <when>` | only Sessions/Projects touched since a duration (`3d`, `2w`, `1h`) or ISO date (`2026-05-01`) | all |

The default scope merges Sessions from both Harnesses by their encoded working
directory. `projects` shows one row when both Harnesses used the same directory.
Codex subagent Sessions stay excluded unless you pass `--include-subagents`.
`sessions` and `projects` are inventory verbs: they keep listing and counting
the Current Session.

## Output and exit codes

- Output is colorized on a TTY and plain when piped or under `NO_COLOR`.
- "No matches" / "No sessions" / "No projects" exit **0** — an empty result is
  not an error.
- Bad input (unparseable regex, unparseable `--since`, an ambiguous id, a
  missing Session, unavailable or ambiguous current context) exits **non-zero**
  with a readable message.

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
