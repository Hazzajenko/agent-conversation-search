# search output carries the session-id + turn handoff

Status: ready-for-agent

## What to build

Turn search output from a standalone snippet list into the **index half of the search→show workflow** (ADR 0002). Search stays a terse index — one line per Match — and explicitly does **not** grow surrounding context; that is `show`'s job.

- Each Session header leads with the **short session-id** (e.g. first 8 chars of the stem) so it can be copied straight into `show`.
- Each Match line is prefixed with its **turn number** (`[12]`) — the same numbering `show --around` consumes.
- The `… +N more` line becomes an **actionable hint**: `+N more  ›  ccsearch show <id>`.

## Acceptance criteria

- [ ] Each Session header shows the short session-id alongside the existing `title · date · branch`
- [ ] Each Match line shows its turn number
- [ ] The overflow line suggests the concrete `ccsearch show <id>` command
- [ ] Snippets stay one line — no surrounding-turn context is added
- [ ] Updated `assert_cmd` output tests

## Blocked by

- 05-output-polish
- 07-show-verb-transcript
