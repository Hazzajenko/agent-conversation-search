# `sessions`: list Sessions in a scope for the show handoff

Status: ready-for-agent
Category: enhancement

## Why

The only way to reach a `show`-able Session today is to `search` for text you remember being in it — there is no content-agnostic "what conversations do I have here?" listing. `sessions` fills that gap in the search→show workflow (ADR 0002): it enumerates Sessions and emits the same short-id header that feeds the stateless session-id handoff, so you can eyeball recent conversations and `show` one without recalling its contents.

Verb-naming and the separate-verb-per-unit choice are recorded in **ADR 0004**. This issue is `sessions` only; `projects` is issue 17.

## Agent Brief

**Category:** enhancement

**Summary:** Add a `sessions` verb that lists the Sessions in a scope, one per line, using the existing Session header so each row is paste-able into `show`. Default scope is the current Project; `--all` / `--project` / `--since` compose exactly as they do for `search`.

**Current behavior:**
`ccsearch` has two verbs (ADR 0002): `search` (default, find a Session by Query) and `show` (render a Session as a Transcript). Both are welded by the short session-id. There is no way to enumerate Sessions without running a text Query — `search` requires a Query, and `-l/--files` only prints paths of Sessions that *matched* a Query.

**Desired behavior:**
- `ccsearch sessions` lists the Sessions in scope, **one row per Session**, where each row is the existing `session_header` verbatim: `short-id · project · title · date · branch` (with the `(untitled)` fallback, and date/branch omitted when absent — exactly as `search` renders its Session header).
- **Default scope is the current Project** (same resolution as a bare `search`, ADR 0001). `--all`, `--project <substr>`, and `--since <when>` compose identically to `search`.
- **Sorted recency-descending** on the same `timestamp` key `search` already sorts on (newest Session first; ties by path). Undateable Sessions sort last; `--since` excludes them (existing rule, issue 12).
- **Inclusion rule:** every `.jsonl` from which a `sessionId` can be read is listed; **no content filtering** (a Session with zero Messages still appears). A file with no extractable `sessionId` is skipped — a row you can't `show` is useless.
- `-l/--files` is supported on `sessions` too: prints only the session file paths, in listing order, for piping into `ccsearch show -`.
- Flags that have no meaning for a listing are **rejected** (clap `conflicts_with` / not wired): `--session`, the content flags (`--thinking`, `--tools`, `--all-content`), and `--failed` / `--full` / `--stats`.
- Empty scope prints `No sessions.\n` and **exits 0** (mirrors `search`'s no-match → exit 0; "nothing here" is not an error).
- Output honours the existing color machinery (anstream/owo-colors): the short-id coloured the same way `search` colours it; no ANSI when piped / `--no-color`.

**Key interfaces (by contract, not location):**
- `session_header(short, project, title, timestamp, branch)` already produces the exact row — reuse it; do **not** fork a second header format. The invariant "a `sessions` row is byte-identical to a `search` Session header line" should hold.
- `enumerate_sessions(project_dirs)` already walks scope into `(sessionId, path)` pairs; the per-Session metadata parse (title/timestamp/branch) already exists for `search`. `sessions` is: resolve scope → enumerate → parse lightweight metadata → sort recency-desc → format header per row. Reuse the scope-resolution path `search` uses so `--all`/`--project`/`--since` behave identically by construction.
- `format_paths` already renders the `-l` path list — reuse for `sessions -l`.
- The `timestamp` field (the recency key, sorted at the existing `b.timestamp.cmp(&a.timestamp)` site) is the sort and `--since` basis — no new recency notion.

**Acceptance criteria:**
- [ ] `ccsearch sessions` with no Query lists current-Project Sessions, newest first, one `short-id · project · title · date · branch` line each (CLI test against a fixture Store).
- [ ] A listed row's short-id round-trips: `ccsearch sessions` then `ccsearch show <that-id>` resolves the same Session.
- [ ] `--all`, `--project <substr>`, and `--since <when>` change the listed set exactly as they change `search`'s scope (test at least `--all` widening and `--since` excluding an older Session).
- [ ] `sessions -l` prints only paths, in the same order, and `ccsearch sessions -l | ccsearch show -` works.
- [ ] A Session with no `ai-title` Record renders `(untitled)`; an undateable Session sorts last and is excluded by `--since`.
- [ ] A `.jsonl` with no extractable `sessionId` is silently skipped (not rendered as a broken row).
- [ ] `--session`, `--thinking`/`--tools`/`--all-content`, and `--failed`/`--stats` are rejected for `sessions` (clap error, non-zero exit).
- [ ] Empty scope prints `No sessions.` and exits 0; piped output carries no ANSI.
- [ ] `cargo test` green and `cargo clippy --all-targets -- -D warnings` clean.

**Out of scope:**
- The `projects` verb (issue 17) — including any directory de-duplication.
- Any per-row count (Messages/Records) or first-Prompt preview — rejected in ADR 0004; the Title + date is the navigation signal.
- A result index / MRU (`show 2`) — forbidden by ADR 0002 ("the Store is not a cache"); the handoff stays the stateless session-id.
- Paging — `sessions` is a pure text emitter (ADR 0002); rely on `--since` and `| more` to bound output.

## Comments
