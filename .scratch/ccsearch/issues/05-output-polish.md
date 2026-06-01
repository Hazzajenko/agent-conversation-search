# Output polish

Status: done

## What to build

Turn the slice-1 basic output into the final readable, scannable format.

- **Recency sort**: Sessions ordered most-recent-first (by newest Record timestamp, or file mtime as a cheap proxy).
- **Full Session header**: `project · title · date · branch`, where date is the `YYYY-MM-DD` prefix sliced off the timestamp (no date library) and branch comes from `gitBranch`.
- **Snippets**: each snippet truncated to a readable width (~200 chars) **centered on the Match**, with the matched span highlighted and `…` ellipses where text is cut.
- **Per-Session cap**: show at most N Matches per Session (default 3) with a `… +N more matches` line; `-m` / `--max-per-session` overrides, and `-m 0` means unlimited.
- **Path mode**: `-l` / `--files` prints only the matching `.jsonl` file paths (grep `-l` style) for piping.
- **Color**: highlight via `anstream` + `owo-colors`, auto-respecting `NO_COLOR` and TTY detection (no manual color flag plumbing).

## Acceptance criteria

- [x] Sessions are printed most-recent-first
- [x] Each Session header shows project, Title, `YYYY-MM-DD` date, and git branch
- [x] Snippets are centered on the Match, truncated to a readable width, with the Match highlighted and ellipses where cut
- [x] At most 3 Matches per Session by default, with a `… +N more` indicator; `-m` overrides and `-m 0` is unlimited
- [x] `-l` / `--files` prints only matching file paths
- [x] Color is applied on a TTY and suppressed under `NO_COLOR` or when piped
- [x] Covered by unit tests plus `assert_cmd` end-to-end tests (including a `NO_COLOR` / piped case)

## Blocked by

- 01-walking-skeleton-current-project-search
