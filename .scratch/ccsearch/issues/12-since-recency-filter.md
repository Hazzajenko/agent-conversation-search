# `--since`: filter Sessions by recency

Status: ready-for-agent

## What to build

Add `--since <duration|date>` as a **general** filter (not specific to `--failed`) so "lately" is expressible. Recency is the dominant relevance axis for conversation history — a Failure or discussion from months ago is almost always noise.

- Accepts a relative duration (`3d`, `2w`, `1h`) or an absolute ISO date (`2026-05-01`).
- Filters at the **Session level** by the Session's recency timestamp (the newest Record timestamp already computed for sorting) — predictable: "Sessions touched since X". Per-Record / per-Failure time filtering is out of scope for v1.
- Composes with every scope and with `--failed`: `ccsearch --failed --since 3d`, `ccsearch "regex" --all --since 1w`.

## Acceptance criteria

- [ ] `--since` parses relative durations (`3d`, `2w`, `1h`) and absolute ISO dates
- [ ] Sessions whose recency timestamp is older than the cutoff are excluded
- [ ] Composes with `--all` / `--project` / `--session` / `--failed`
- [ ] Sessions lacking a timestamp are excluded (behaviour documented)
- [ ] Unit tests for the duration / date parser and the filtering

## Blocked by

- 05-output-polish
