# Unit-listing verbs: `sessions` and `projects` as noun-verbs

ADR 0002 established the action-verb model — `search` (find a Session) and `show` (read a Session) — welded together by one identifier, the short session-id. A third job then emerged: *enumerate* what exists, content-agnostically, so you can reach a Session you don't remember the contents of. Today the only path to a `show`-able session-id is to `search` for text you recall being in it; there is no "what conversations do I have here?" listing.

**Decision:** Add listing verbs named after the **unit** they enumerate, not the action: **`agsearch sessions`** lists Sessions, and a future **`agsearch projects`** lists Projects. They are **separate verbs**, not one verb with a unit-switching flag. Each `sessions` row is the existing Session header from ADR 0002 (`short-id · project · title · date · branch`) verbatim, so the listing feeds the same stateless session-id → `show` handoff that `search` and `--failed` already feed. `sessions` defaults to the current Project and composes with `--all` / `--project` / `--since` exactly like `search`.

## Considered and rejected

- **A generic `list` verb** (lists Sessions by default). Reads cleanly as an action-verb (consistent with `search`/`show`), but `list` alone is ambiguous — *list what?* — and it forces the future Projects lister into a lopsided sibling (`list` for Sessions, `projects` for Projects). Rejected: naming the verb after the unit is self-documenting and scales symmetrically (`sessions`, `projects`).
- **One `list` verb with a `--projects` flag** to switch the unit. Fewer verbs, but a flag that changes *what kind of thing* is listed is the unit/scope overloading ADR 0002 warns against, and it couples two independent jobs. Rejected in favour of one verb per unit.
- **A Match/Record count column** on each row. Adds navigation richness but requires reading and counting every Record in every in-scope Session, and introduces a column `search` doesn't have. Rejected: the Title + date is enough navigation signal, and row-equals-search-header keeps one format across the tool.

## Consequences

- The tool now mixes action-verbs (`search`, `show`) with noun-verbs (`sessions`, `projects`). This is a deliberate, documented inconsistency: actions are verbs, unit-enumerations are the plural noun. This ADR exists so that mix doesn't read as an accident.
- `sessions` introduces **no new domain term** — it enumerates the existing **Session**, so CONTEXT.md is unchanged.
- `projects` is deferred. It inherits this verb model, but carries an unresolved sub-design: per ADR 0001 a single logical Project can map to several directories (drive-letter case wobble, lossy encoding collisions), so a naive listing double-counts. Its de-duplication rule is its own triage question.
