# Walking skeleton — substring search of the current Project

Status: done

## What to build

The thinnest runnable version of `ccsearch`. Running `ccsearch "query"` from inside a directory finds the conversations Claude Code recorded for that directory and prints where the Query matched.

End-to-end behavior:

- Resolve the **Store** (the root holding all Projects) with precedence `--claude-dir <path>` > `$CLAUDE_CONFIG_DIR` > `~/.claude`. This precedence exists from the start because it is the seam that lets end-to-end tests point at a fixture Store.
- Encode the current working directory to a Project directory name by replacing **every non-alphanumeric character with `-`** (see ADR-0001).
- Match that encoded name **case-insensitively** against the directories in `<store>/projects/`. If several match, search the **union** of them.
- Scan each `.jsonl` Session file in the matched Project(s), parsing Records leniently: an unparseable or unknown-type line is **skipped, never fatal**.
- Match a **case-insensitive literal substring** across the **default content set**: user **Prompts**, assistant **Replies** (`text` blocks only), and Session **Titles** (`ai-title`). Thinking and tool content are excluded.
- Print **Matches grouped by Session**, each Session with a basic header (project name · Title) and, under it, each matching Record's role plus a truncated snippet of the matched text. Output must be readable by both a human and Claude.

Establishes the **library + binary split**: all logic lives in the lib (pure functions, typed errors via `thiserror`, unit-tested against `tempfile` fixture Stores); `main.rs` is thin `clap` + `anyhow` wiring.

## Acceptance criteria

- [ ] `ccsearch "term"` run inside a directory prints Matches from that directory's Project only
- [ ] Store resolves via `--claude-dir`, then `$CLAUDE_CONFIG_DIR`, then `~/.claude`
- [ ] The cwd→directory encoder replaces every non-alphanumeric character with `-` and is covered by `proptest`
- [ ] Project directory lookup is case-insensitive and unions multiple matches
- [ ] Default content set is Prompts + Replies + Titles; thinking and tool content are not matched
- [ ] Matching is literal substring, case-insensitive
- [ ] Malformed / unknown-type Records are skipped without aborting the search
- [ ] Output is grouped by Session with a header and per-Match snippet
- [ ] Logic lives in the lib with unit tests; at least one `assert_cmd` end-to-end test drives the binary against a `tempfile` fixture Store
- [ ] A Query with no Matches exits cleanly with a clear "no matches" indication

## Blocked by

None - can start immediately
