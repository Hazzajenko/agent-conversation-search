# Scope widening — `--all` and `--project`

Status: ready-for-agent

## What to build

Let the user search beyond the current Project.

- `--all` searches every Project in the Store.
- `--project <substr>` searches Projects whose **name matches the substring case-insensitively**. On multiple matches, search the **union** (consistent with ADR-0001). A full Project path works because it contains itself as a substring.
- `--all` and `--project` override the default current-Project scoping.
- Fan out the file scan across Sessions with `rayon` so a full `--all` sweep of the whole Store stays fast.

## Acceptance criteria

- [ ] `ccsearch --all "term"` returns Matches from every Project in the Store
- [ ] `ccsearch --project creature "term"` matches Project names case-insensitively and unions multiple matches
- [ ] Explicit scope flags override the default current-Project behavior
- [ ] File scanning is parallelized across Sessions
- [ ] Results remain correct and deduplicated when a Project is matched more than once
- [ ] Behavior is covered by unit tests over fixture Stores plus an `assert_cmd` end-to-end test

## Blocked by

- 01-walking-skeleton-current-project-search
