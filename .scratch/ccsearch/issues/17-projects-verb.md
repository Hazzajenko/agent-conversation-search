# `projects`: list the Projects in the Store

Status: needs-triage
Category: enhancement

## Why

Sibling to `sessions` (issue 16) under the unit-listing verb model (ADR 0004): enumerate the Projects in the Store so you can see what history exists and what substring to pass to `--project`. Lower-value than `sessions` (you usually know your own project names), but the natural completion of the listing pair — most useful to `--all` users exploring across projects.

## The open sub-design (why this is needs-triage, not ready-for-agent)

ADR 0001: a single **logical Project can map to several directories** under `~/.claude/projects/` — drive-letter case wobble on Windows (`E--…` vs `e--…`) and the lossy non-alphanumeric→`-` encoding (`a\b\c`, `a-b-c`, `a.b.c` all collapse). `--project` already handles this by searching the **union** of matching directories. A naive `projects` listing would **double-count** the same logical Project as two rows.

So the core decision to resolve at triage:

1. **List directories raw** — one row per `~/.claude/projects/` subdirectory. Honest about what's on disk, but shows duplicates the rest of the tool deliberately unions away. Inconsistent with `--project`.
2. **List de-duplicated logical Projects** — collapse case-variant / encoding-collision directories into one row (the same union rule `--project` uses). Consistent with how the tool already treats a Project, but needs a canonical display name and a defined merge of their metadata (counts, last-touched).
3. Something else surfaced at triage.

Leaning option 2 (consistency with ADR 0001's union), but it forces sub-questions: what is the **display name** of a logical Project (the encoded directory string? a real `cwd` read out of a Record, per ADR 0001's "read the cwd field"?), and when two directories merge, how do their **last-touched dates / session counts** combine?

## Likely shape (to firm up at triage)

- `ccsearch projects` lists Projects, newest-touched first, reusing as much of the `sessions`/`search` scope and recency machinery as fits.
- Each row likely: a Project display name + session count + last-touched date. (Row format is a triage decision — unlike `sessions`, there is no existing header to reuse verbatim.)
- `--since` should compose (Projects touched since X). `--all` is implicit (the whole Store); `--project <substr>` could filter the listing.

## Out of scope

- Verb naming / the action-vs-noun-verb question — already decided in ADR 0004.
- Anything about `sessions` (issue 16).

## Blocked by

- 16-sessions-verb (shares scope/recency plumbing; build `sessions` first)

## Comments
