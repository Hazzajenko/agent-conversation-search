# Match modes — `--regex` and `--case-sensitive`

Status: done

## What to build

Add the two non-default ways to match a Query, routed through a single `regex`-based match path.

- Default stays literal substring, case-insensitive.
- `--regex` / `-e` treats the Query as a regular expression.
- `--case-sensitive` / `-s` makes matching case-sensitive.
- Literal mode is implemented by escaping the Query (`regex::escape`) so there is exactly one matching code path for both modes.
- An invalid regex produces a clear error message and a non-zero exit, not a panic.

## Acceptance criteria

- [x] `--regex` matches the Query as a regular expression
- [x] `--case-sensitive` makes both literal and regex matching case-sensitive
- [x] Default (no flags) remains literal, case-insensitive
- [x] Literal and regex modes share one match path via `regex::escape`
- [x] An invalid regex exits non-zero with a readable error and no panic
- [x] Each mode/case combination is covered by unit tests plus an `assert_cmd` end-to-end test

## Blocked by

- 01-walking-skeleton-current-project-search
