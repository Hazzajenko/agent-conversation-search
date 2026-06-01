# `--session` scope: search within one Session

Status: ready-for-agent

## What to build

Add `--session <id-prefix>` as a fourth scope, alongside the default current-Project, `--all`, and `--project`. It restricts the search to a single Session — the "where in this conversation" step of the find → locate → read workflow (e.g. `ccsearch "chose|instead" --session 4c28878f`).

- Resolves the prefix across the **whole Store**, identically to `show`.
- **Query stays required** — `--session <id>` with no Query is *not* "dump the Session" (that is `show`'s job).
- Conflicts with `--all` and `--project` (the scopes are mutually exclusive).

## Acceptance criteria

- [x] `ccsearch "x" --session <prefix>` searches only that Session, printing turn-numbered Matches
- [x] The prefix resolves across the whole Store; ambiguous / no-match errors cleanly
- [x] Omitting the Query together with `--session` is a usage error
- [x] `--session` conflicts with `--all` and `--project`
- [x] Unit plus e2e tests

## Blocked by

- 09-search-index-handoff
