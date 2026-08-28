# One parser per Harness, one shared internal model

ADR 0006 concentrated all knowledge of the on-disk format into a single
`session` module producing typed `Record`s, with every verb a projection over
`&[Record]`. Codex support introduces a second on-disk format — a different
Store layout (date-partitioned rollout files vs per-project directories), a
different Record envelope (`{timestamp, type, payload}` vs typed lines), and
different derivations for Project, Title, and Failure. The question is where the
second format lives.

**Decision:** ADR 0006's "one parser" becomes **one parser per Harness, one
shared internal model**. Each Harness implements a small adapter with three
responsibilities: discover its Store, enumerate Sessions (with Project key,
Title, and timestamps), and parse a Session file into the existing typed
`Record` model. Everything downstream of the adapter — search, show, listing,
failure analysis — stays Harness-agnostic and unchanged: projections over
`&[Record]` that never see a Harness-specific shape. The design anticipates
further Harnesses; adding one means adding one adapter, touching no projection.

## Considered and rejected

- **Normalise Codex files to the Claude format on read.** Rejected: it forces
  Codex concepts through a foreign envelope, inventing fake `ai-title` Records
  and `is_error` flags. The shared model should be *our* typed `Record`, not one
  Harness's wire format.
- **Parallel Codex projections (a second search, a second show).** Rejected for
  the reason ADR 0006 exists: the verbs' behaviour (turn numbering, snippet
  windowing, grouping) must not fork per Harness, or every invariant becomes a
  cross-checked coincidence again.
- **A single parser with per-Harness branches inside.** Workable at two
  Harnesses, but each branch touches every match arm; the adapter seam keeps
  each format's knowledge in one file and makes "add a Harness" additive.

## Consequences

- The typed `Record`/`Block` model is promoted from "the Claude format, typed"
  to the tool's internal language; Codex constructs map into it (e.g.
  `custom_tool_call` → tool use, `reasoning` → thinking).
- Where the model needs a concept one Harness lacks (Codex Failure has no
  structural flag), the *adapter* owns the inference, so projections keep a
  single Failure semantics. Codex failure inference may lag search/show —
  parity is delivered incrementally.
- Subagent-thread exclusion (CONTEXT.md, Session) is enforced at enumeration
  time by the Codex adapter; projections never see excluded files.
- ADR 0002's transparent `sessionId` handoff extends across Stores: lookup
  tries every Harness's enumeration; UUIDs make collisions a non-concern.
