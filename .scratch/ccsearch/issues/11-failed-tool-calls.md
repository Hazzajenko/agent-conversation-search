# `--failed`: list failed tool calls by structure

Status: ready-for-agent

## What to build

Add `--failed`, a flag on search that finds **Failures** (see CONTEXT.md) by *structure* instead of by Query: `tool_result` Records with `is_error: true`, joined via `tool_use_id` back to the originating `tool_use` for the tool name and command.

- Output reuses the search index shape (Session grouping, short id, turn numbers, `+N more › show` hint). Each Failure renders as two lines:
  ```
  [turn] ✗ <tool>  <command>
         exit <code> · <salient error line>
  ```
- **Salient line** = the first match against a **fixed, documented marker list** (`error[`, `panicked at`, `assertion failed`, `Error:`, `does not exist`, `unexpected EOF`, …) applied in priority order, with a **last-non-empty-line fallback** when none match. ANSI codes stripped. This is predictable and documented — a lookup table, *not* a relevance ranker. (Real data showed the naive "first line" is useless: it's just `Exit code 101`, while the real `error[E0433]` is several lines down.)
- `--full` prints the entire error text inline instead of the salient line.
- **Query optional**: `ccsearch --failed` lists all Failures in scope; `ccsearch "cargo" --failed` lists only Failures whose command / error matches the Query.
- Inherits scope (`--all`, `--project`, `--session`) and the `-m` per-Session cap, so `ccsearch --failed` defaults to the current Project, 3 per Session.

## Acceptance criteria

- [ ] `--failed` lists Failures joined `tool_use` ↔ `tool_result`, grouped like search with id + turns + show hint
- [ ] The salient line uses the fixed marker list with a last-line fallback; ANSI is stripped
- [ ] `--full` shows the complete error text inline
- [ ] Query is optional under `--failed` and filters Failures when present
- [ ] Respects scope and `-m`
- [ ] Unit tests for the join and the salient-line picker over fixtures (compiler error, panic, bash EOF, missing file); plus an e2e test

## Blocked by

- 09-search-index-handoff
