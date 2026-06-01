# Content selection — `--thinking`, `--tools`, `--all-content`

Status: done

## What to build

Let the user widen what gets searched beyond the slice-1 default set (Prompts + Replies + Titles).

- `--thinking` also searches assistant `thinking` blocks.
- `--tools` also searches tool calls and tool results (`tool_use` blocks and user `tool_result` content).
- `--all-content` searches everything (equivalent to enabling all opt-in content).
- Flags compose (e.g. `--thinking --tools`).
- Each Match is still attributed to the correct content kind so output stays legible (e.g. a thinking Match is labelled as such, not as a Reply).

## Acceptance criteria

- [x] Default run (no content flags) searches only Prompts, Replies, and Titles
- [x] `--thinking` includes thinking blocks in matching
- [x] `--tools` includes tool calls and tool results in matching
- [x] `--all-content` includes every content kind
- [x] Content flags compose correctly
- [x] Matches are attributed to their content kind in output
- [x] Covered by unit tests over fixtures plus an `assert_cmd` end-to-end test

## Blocked by

- 01-walking-skeleton-current-project-search
