# The `search` / `show` verb-vs-query collision

`ccsearch` will not change its argument grammar to make the bare verb words
(`search`, `show`) directly searchable, and it will not require an explicit
`search` verb. The current behaviour stands:

- `ccsearch <query>` searches — `search` is the **default** verb (ADR 0002), so
  the everyday case skips the verb entirely (`ccsearch tokio`).
- `ccsearch search` (one word) is parsed as the `search` *command* with no
  query, and errors with "a query is required".
- `ccsearch search search` (two words) searches for the literal word "search".

So a token that happens to equal a verb name is still reachable as a Query —
just via the explicit verb form. Nothing is unsearchable.

## Why this is out of scope

Two changes were proposed to "fix" the collision; both were rejected.

**1. Special-case the collision so `ccsearch search` searches for "search".**
This would require the bare positional to mean "Query" *except* when it equals a
verb name, *except* when the user actually wanted the verb — an unresolvable
ambiguity. ADR 0002 explicitly rejected overloading the positional to be "a
Query *or* a session-id" for the same reason: collapsing two roles into one
positional is the confusion the tool is built to avoid. The verb words are no
different.

**2. Require the explicit `search` verb for every search (drop the bare
default).** This was considered and declined because:

- It doesn't even fix the thing it's meant to. You would *still* type
  `ccsearch search search` to find "search", and `ccsearch search` would *still*
  error — the verb-word-as-query quirk is untouched.
- It taxes the common case to defend against a rare one. Searching is what the
  tool does 95% of the time; peer tools (`grep`, `rg`, `ag`) take the pattern as
  the bare argument and don't make you name a verb. Forcing `ccsearch search foo`
  on every invocation is friction on the hot path.
- It reverses a deliberate, documented decision (ADR 0002: "bare
  `ccsearch <QUERY>` still searches — no muscle-memory break").

```text
ccsearch tokio          # everyday search — no verb needed (the default)
ccsearch search tokio   # identical; the explicit verb form
ccsearch search search  # the escape hatch: search for the literal word "search"
ccsearch search         # error: a query is required  ← the accepted quirk
```

The residual rough edge — that a first-time user might type `ccsearch search`
expecting it to search for "search" — is a momentary surprise, not a defect.
The appropriate remedy, if any, is a friendlier error message on that exact
path, **not** a change to the grammar. That message tweak was itself judged too
low-value to schedule, but remains the only acceptable direction if this is ever
revisited.

## Prior requests

- #15 — "A bare Query of `search` or `show` is swallowed by the subcommand"
  (closed wontfix 2026-06-02)
