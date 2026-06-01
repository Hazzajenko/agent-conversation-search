# Verbs and the session-id handoff: search, show, and analyse as one workflow

`ccsearch` began as a single implicit verb — `ccsearch <QUERY>` searches and prints Snippets. Three distinct jobs then emerged: **find** a Session (search), **read** a whole Session (show), and **analyse** failures (`--failed`). Cramming all three into one output is what made search results hard to read and left no way to open a conversation.

**Decision:** Introduce subcommands with **`search` as the default verb** (bare `ccsearch <QUERY>` still searches — no muscle-memory break) and **`show <session>` as an explicit sibling** that renders a Transcript. All three jobs are welded together by **one identifier — the short session-id** (git-style unique prefix, resolved across the whole Store) — and **one coordinate — the turn number**. `search` and `--failed` emit `id` + `turn`; `show <id> --around <turn>` reads the neighbourhood. `--failed` is a *flag on search*, not its own verb, because a Failure is just "a Match found by structure instead of by a Query."

## Considered and rejected

- **Result index / MRU (`show 1`).** Ergonomic to type, but it forces persistent state between two separate process runs — contradicting the "Store is not a cache" principle in CONTEXT.md — and "last search" is ambiguous (which cwd?) and goes stale the moment you search again. Rejected in favour of a stateless session-id prefix.
- **Built-in pager** (git-style auto-paging when stdout is a TTY). Beloved in git *because git ships its own pager*; on Windows/PowerShell `less` usually isn't installed and `more` is weak, so cross-platform pager spawning is fragile. Rejected — `show` stays a pure text emitter, and windowing (`--around`) keeps output small enough that `| more` is the rare escape.
- **Overloading the positional** (one argument that is a Query *or* a session-id). Collides Query with session-id — exactly the confusion CONTEXT.md warns against — and breaks the day a Query looks like a hex prefix. Rejected: session-id is always a flag / `show` argument; Query is always the positional.

## Consequences

Search output must now carry a short session-id header and per-Match turn numbers (issue 09) to feed the handoff. The aggregate failure-counts report (`stats`, issue 13) is deliberately deferred until `--failed` reveals what "similar failures" should mean.
