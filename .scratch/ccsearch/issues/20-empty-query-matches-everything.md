# An empty query matches every Record instead of erroring

Status: ready-for-agent (migrated to GitHub issue #2)
Category: bug

## Why

`ccsearch ""` dumps the entire Project (exit 0) — the empty string is a valid
substring of every Record, so the literal matcher matches all. A missing query
already errors cleanly (`ccsearch: a query is required …`), so an empty query
behaving completely differently is a footgun: a shell variable or command
substitution that expands to empty silently dumps everything instead of failing
fast.

```console
$ ccsearch ""        # prints every session in the Project, exit 0
$ ccsearch           # ccsearch: a query is required …, exit 1
```

## Agent Brief

**Category:** bug

**Summary:** Treat an empty (or whitespace-only?) Query the same as a missing
Query: print the existing "a query is required" message and exit non-zero,
rather than matching every Record.

**Current behavior:**
`run_search` (main.rs:319) compiles `Some(query)` into a `Matcher` whenever the
query is `Some(_)`; `Some("")` compiles to a matcher that matches everything.
The empty case is never distinguished from a real query.

**Desired behavior:**
- A `search` with an empty-string Query errors with the same message and
  non-zero exit as a missing Query.
- Decide in triage whether whitespace-only (`"   "`) is also rejected, or only a
  truly empty string. (Leaning: reject empty only; a user who quotes spaces may
  mean it. Confirm.)
- `--failed ""` (Query is the optional failure filter) — decide whether an empty
  filter means "no filter" (current effect) or is rejected. Keep them
  consistent or document the difference.

**Acceptance criteria:**
- [ ] `ccsearch ""` exits non-zero with the "query is required" message (CLI
      test).
- [ ] A normal one-character query still works (regression).
- [ ] Behaviour of empty `--failed` filter decided and tested.

**Out of scope:**
- Any change to non-empty matching semantics.

## Comments

### 2026-06-04 — Filed

Found during edge-case testing. Minor severity (you have to actively pass an
empty string) but a classic scripting footgun; cheap to fix. `needs-triage` only
for the whitespace-only and empty-`--failed`-filter policy calls.

### 2026-08-27 — Migrated

Triaged and migrated to GitHub issue #2 (ready-for-agent). Bug reproduced. Maintainer decisions: reject empty-string query only (whitespace-only stays valid); empty --failed filter keeps meaning "no filter". This file is read-only history.
