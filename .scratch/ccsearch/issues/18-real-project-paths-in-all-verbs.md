# Show the real project path (cwd) in every verb, not just `projects`

Status: ready-for-agent
Category: bug

## Why

The README's stated reason this tool exists is that Claude Code's on-disk
directory names are mangled (`E:\projects\rust` → `E--projects-rust`). Yet only
the `projects` verb recovers the real path. `search`, `sessions`, and
`--failed`/`--stats` all display the encoded directory name in their Session
headers:

```
projects → E:\projects\rust\claude-code-conversation-search   ← real path ✓
sessions → E--projects-rust-claude-code-conversation-search   ← mangled ✗
search   → E--projects-rust-claude-code-conversation-search   ← mangled ✗
--failed → e--Vault2026                                        ← mangled ✗
```

The README itself advertises real paths in its `search` and `sessions` examples
(`E:\projects\rust\demo`), so this is also a docs/behaviour mismatch. ADR 0005
already sanctions reading the `cwd` field purely as a **display label** (it is
about Project *identity/grouping*, not about forbidding the real path
elsewhere), so this change is consistent with existing decisions — the unit
matched by `--project` stays the encoded directory; only the displayed label
changes.

## Agent Brief

**Category:** bug

**Summary:** Make every Session-header-emitting verb display the real working
directory (`cwd`) as its project label, falling back to the encoded directory
name when no Session carries a `cwd` — exactly as `projects` already does
(`newest.cwd.unwrap_or_else(|| newest.project)`, lib.rs:466).

**Current behavior:**
- `SessionInfo` already carries `cwd` (lib.rs:413), but `format_sessions`
  (lib.rs:615) passes `s.project` (the encoded name) to `session_header`.
- `SessionMatches` (lib.rs:236) and `SessionFailures` (lib.rs:1233) have **no
  `cwd` field at all**, so `format_results` and `format_failures` structurally
  cannot show the real path.

**Desired behavior:**
- The project column in `sessions`, `search`, and `--failed`/`--stats` Session
  headers shows the Session's real `cwd` when known, else the encoded directory
  name (the existing fallback contract).
- Per-Session cwd (each header shows its own Session's recorded cwd) — strictly
  more correct than `projects`' "newest Session's cwd for the whole directory
  group", and the natural fit since these verbs already render per Session.
- `--project <substr>` and scope resolution are **unchanged** — they still match
  the encoded directory name (ADR 0001 / ADR 0005: the display label may differ
  from the matched unit; they agree on alphanumeric runs).

**Key interfaces:**
- `session_header(short, project, title, timestamp, branch)` is the single
  header formatter — feed it `cwd.as_deref().unwrap_or(&project)` from each call
  site rather than forking the formatter.
- `SessionInfo` fix is a one-liner (the field already exists). `SessionMatches`
  and `SessionFailures` each need a `cwd` field populated where their metadata is
  read (the same `session::read(&text).meta` pass that already yields
  title/timestamp/branch carries `cwd`).

**Acceptance criteria:**
- [ ] `sessions` shows the real cwd when present, encoded name as fallback (CLI
  test against a fixture Store with and without a `cwd` Record).
- [ ] `search` and `--failed` headers do the same; `--stats` is unaffected (it
  keys on tool+signature, not project name).
- [ ] `--project <substr>` still selects by encoded directory name (regression
  test: a substring of the encoded name still matches).
- [ ] README `search`/`sessions` examples now match real output.
- [ ] `cargo test` green, `cargo clippy --all-targets -- -D warnings` clean.

**Out of scope:**
- Changing Project identity to be cwd-keyed (rejected in ADR 0005).
- The `projects` verb (already correct).

## Comments

### 2026-06-04 — Filed

Found while dogfooding every verb against the real Store. `sessions`-only fix
landed first (see issue, it was a one-liner reusing the existing `cwd` field);
the `search` / `--failed` half remains because their result structs need a new
`cwd` field threaded through. This issue tracks the full set; mark done when
`search` and `--failed` also show real paths.
