# `sessions`: list Sessions in a scope for the show handoff

Status: done
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
- [x] `ccsearch sessions` with no Query lists current-Project Sessions, newest first, one `short-id · project · title · date · branch` line each (CLI test against a fixture Store).
- [x] A listed row's short-id round-trips: `ccsearch sessions` then `ccsearch show <that-id>` resolves the same Session.
- [x] `--all`, `--project <substr>`, and `--since <when>` change the listed set exactly as they change `search`'s scope (test at least `--all` widening and `--since` excluding an older Session).
- [x] `sessions -l` prints only paths, in the same order, and `ccsearch sessions -l | ccsearch show -` works.
- [x] A Session with no `ai-title` Record renders `(untitled)`; an undateable Session sorts last and is excluded by `--since`.
- [x] A `.jsonl` with no extractable `sessionId` is silently skipped (not rendered as a broken row).
- [x] `--session`, `--thinking`/`--tools`/`--all-content`, and `--failed`/`--stats` are rejected for `sessions` (clap error, non-zero exit).
- [x] Empty scope prints `No sessions.` and exits 0; piped output carries no ANSI.
- [x] `cargo test` green and `cargo clippy --all-targets -- -D warnings` clean.

**Out of scope:**
- The `projects` verb (issue 17) — including any directory de-duplication.
- Any per-row count (Messages/Records) or first-Prompt preview — rejected in ADR 0004; the Title + date is the navigation signal.
- A result index / MRU (`show 2`) — forbidden by ADR 0002 ("the Store is not a cache"); the handoff stays the stateless session-id.
- Paging — `sessions` is a pure text emitter (ADR 0002); rely on `--since` and `| more` to bound output.

## Comments

### 2026-06-02 — Done

Implemented the `sessions` verb. `list_sessions(project_dirs)` is the content-agnostic counterpart to `search_project_dirs`: it enumerates Sessions via the shared `enumerate_sessions`, gathers header metadata (Title/timestamp/branch) per file with no Query, and sorts by the same recency key (`b.timestamp.cmp(&a.timestamp)`, ties by path). A new `SessionInfo` carries exactly the fields `session_header` needs, so `format_sessions` emits rows byte-identical to a search Session header (test asserts the exact string). `format_session_paths` backs `sessions -l`. CLI: a `sessions` subcommand with a `SessionsArgs` that defines only `--all`/`--project`/`--since`/`-l`; the search-only flags it omits are rejected by clap for free. Refactored the scope builder into a shared `build_scope` and the `--since` parse into a shared `parse_since` (search now reuses both).

**Verified on the real Store:** `sessions` lists this project's 7 conversations newest-first; the top row's short-id round-trips through `show`; `--all` widens to 190; `--since` composes; `sessions -l | ccsearch show -` reopens a transcript; `--failed` is rejected with a clap error. 104 lib + 38 cli tests green, clippy clean.

**Known minor duplication:** `session_info` repeats the metadata-folding loop from `search_one_session` (timestamp/branch/title). Not shared, because search does it in the same pass as matching + turn-counting; extracting it would either double-iterate segments in the hot search path or need a wider return. Left as-is deliberately; a candidate for a later `simplify` pass if a third caller appears.
