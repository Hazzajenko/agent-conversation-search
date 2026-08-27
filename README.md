# ccsearch

Search your local Claude Code conversation history from the command line.

Claude Code records every conversation as a `.jsonl` transcript under
`~/.claude/projects/`. `ccsearch` searches those transcripts — so you can find
the session where you figured something out, reopen a past conversation, or see
what tool calls have been failing — without leaving the terminal.

```console
$ ccsearch "borrow checker"
d712581e · E:\projects\rust\demo · Lifetimes and the borrow checker · 2026-06-02 · main
  [14] user: how do I satisfy the borrow checker here without cloning
  [16] assistant: …the borrow checker is complaining because the reference outlives…
  … +3 more  ›  ccsearch show d712581e

$ ccsearch show d712581e        # reopen the whole conversation
```

## Why

Claude Code has no built-in way to look back over your own history. The
transcripts are on disk, but the directory names are mangled
(`E:\projects\rust` → `E--projects-rust`) and each file is a stream of JSON
records, not something you can skim. `ccsearch` turns that pile into three jobs:

- **find** a conversation — `ccsearch <query>`
- **read** a conversation — `ccsearch show <id>`
- **analyse** what went wrong — `ccsearch --failed` / `--stats`

## Install

Requires a [Rust toolchain](https://rustup.rs/).

```console
# from a clone of this repo
cargo install --path .          # installs the `ccsearch` binary onto your PATH
# or just build it
cargo build --release           # ./target/release/ccsearch
```

`ccsearch` reads `$CLAUDE_CONFIG_DIR` (falling back to `~/.claude`) to find your
history. Override it per-run with `--claude-dir <PATH>`. Note that Windows and
WSL keep **separate** histories — run `ccsearch` from the environment whose
sessions you want to search, or point `--claude-dir` at the other one.

## The core workflow

Every verb is welded together by one identifier: the **short session-id** (a
git-style unique prefix). `search` and `--failed` print it; `show` resolves it.
That is the whole loop — find something, copy its id, open it:

```console
$ ccsearch "diesel migration"      # find — note the id in each header
$ ccsearch show 4c28878f           # read — the full transcript
$ ccsearch show 4c28878f --around 220   # read — just the turns around turn 220
```

You can also pipe by path instead of copying ids:

```console
$ ccsearch -l "diesel migration" | ccsearch show -   # open the first match
```

## Verbs

### `search` (the default)

`ccsearch <query>` searches the **current directory's** Project for a
case-insensitive substring, grouped by Session, newest first. It is the default
verb, so the word `search` is optional (`ccsearch foo` ≡ `ccsearch search foo`).

| Flag | Effect |
|------|--------|
| `-e`, `--regex` | treat the query as a regular expression |
| `-s`, `--case-sensitive` | match case-sensitively |
| `--thinking` | also search assistant thinking blocks |
| `--tools` | also search tool calls and tool results |
| `--all-content` | search everything (`--thinking --tools`) |
| `-m`, `--max-per-session <N>` | cap matches shown per Session (`0` = unlimited; default 3) |
| `-l`, `--files` | print only matching file paths, for piping |

By default only Prompts, Replies, and Titles are searched — thinking and tool
content are opt-in.

### `show` — read a whole conversation

`ccsearch show <id>` renders a Session as a readable transcript (tool calls
collapse to one-liners; failed tool calls are flagged; thinking is hidden).

| Flag | Effect |
|------|--------|
| `--thinking` | expand assistant thinking blocks |
| `--around <turn>` | show only the turns around this turn (e.g. a search hit) |
| `--context <N>` | turns of context on either side of `--around` (default 3) |

`<id>` is a git-style prefix resolved across your whole history; `-` reads a
file path from stdin.

### `sessions` — list conversations

`ccsearch sessions` lists the Sessions in scope, newest first, one per line —
for when you want to reopen a recent conversation but don't remember anything to
search for. Each row is paste-able into `show`.

```console
$ ccsearch sessions
de151981 · E:\projects\rust\demo · Test app runtime and functionality · 2026-06-02 · main
d712581e · E:\projects\rust\demo · Lifetimes and the borrow checker · 2026-06-02 · main
```

### `projects` — list projects

`ccsearch projects` lists every Project in your history (whole-Store), newest
first, with how many Sessions each holds and when it was last touched. The name
is the real path, recovered from the transcripts.

```console
$ ccsearch projects
E:\projects\rust\claude-code-conversation-search · 7 sessions · 2026-06-02
E:\projects\games\creature-game · 43 sessions · 2026-06-01
```

## Failure analysis

Find and aggregate **failed tool calls** by structure — independent of any
query — to see what's been breaking.

```console
$ ccsearch --failed                # list failed tool calls, joined to the command
$ ccsearch --failed cargo          # filter failures by command / error text
$ ccsearch --stats                 # aggregate into a counts table by tool + error
```

```console
$ ccsearch --stats
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
| `--session <prefix>` | a single Session, by id-prefix | search |
| `--since <when>` | only Sessions/Projects touched since a duration (`3d`, `2w`, `1h`) or ISO date (`2026-05-01`) | all |

## Output and exit codes

- Output is colorized on a TTY and plain when piped or under `NO_COLOR`.
- "No matches" / "No sessions" / "No projects" exit **0** — an empty result is
  not an error.
- Bad input (unparseable regex, unparseable `--since`, an ambiguous id, a
  missing Session) exits **non-zero** with a readable message.

## As a Claude Code plugin

This repo also ships a Claude Code skill (`skills/ccsearch`, declared in
`.claude-plugin/plugin.json`) that lets Claude recall past conversations for you
— "did we ever discuss X?", "find the session where we set up Y". The skill is
thin glue over the same binary, so install `ccsearch` on your PATH first.

## Concepts

The precise vocabulary this tool is built around — **Project, Session, Record,
Message, Query, Match, Failure, Store** — lives in [`CONTEXT.md`](CONTEXT.md).
The reasoning behind the bigger design decisions is in
[`docs/adr/`](docs/adr/) (locating Projects, the verb/handoff model, failure
grouping, the listing verbs).

## License

MIT.
