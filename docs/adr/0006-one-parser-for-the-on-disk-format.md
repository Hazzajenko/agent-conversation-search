# One parser for the on-disk format; projections own drop-policy

The on-disk transcript format — the tool's whole domain (CONTEXT.md) — was
parsed independently by five functions: `search_one_session`, `session_info`,
`failures_in_one_session`, `parse_transcript`, and `segments_from_value`. Each
re-derived where text lives in the JSON, three re-accumulated the same Session
metadata (newest timestamp, first branch, Title, cwd), and two re-implemented the
turn-numbering rule that ADR 0002 makes the load-bearing handoff coordinate. That
rule stayed correct across its two implementations only because a cross-checking
test (`search_and_show_agree_on_turn_numbers`) failed if they drifted — the
invariant was a tested coincidence, not a structural fact.

**Decision:** One module, `session`, owns reading the on-disk format.
`session::read(&str) -> Session` does a single pass: it parses each line into a
typed **Record** (`Prompt` / `UserBlocks` / `Assistant` / `Title`) carrying its
turn number, and accumulates `SessionMeta`. **Block** becomes a named type, with
**distinct user and assistant kinds** so illegal combinations (a user `tool_use`,
an assistant `tool_result`) are unrepresentable. The parser is **lossless** — it
represents what is on disk, including empty blocks such as a signature-only
`Thinking("")` — and applies **no content policy**. Every consumer (`search`,
`--failed`, `show`, `sessions`) is a pure projection over `&[Record]` that
decides for itself what to drop. Turn numbering lives only in `read`, so ADR
0002's invariant holds by construction.

## Considered and rejected

- **Drop empty blocks in the parser.** Rejected: `search` and `show` legitimately
  disagree — `search` emits a harmless, non-matching empty Segment for an empty
  Prompt, while `show` drops it. Drop-policy is therefore a *projection* concern;
  forcing it into the parser would impose one behaviour on both and silently
  change `search`.
- **A shared parser that still yields raw `serde_json::Value` records.** Rejected:
  it would kill the metadata and numbering duplication but leave the JSON-shape
  knowledge (content string-vs-array, `tool_result` content quirks) duplicated
  across every projection — a new abstraction with only partial payoff.
- **Leave the five parsers in place.** Rejected: the format *is* the domain.
  Smearing it across five functions made the turn-number invariant a tested
  coincidence and concentrated nothing — the deletion test for any one of them
  just moved complexity to the others.

## Consequences

- The on-disk shape changes in **one** place. A new Record or Block kind is added
  to the typed model once; every projection then gets a compile error until it
  handles the new variant (exhaustive `match`, no wildcard).
- ADR 0002's turn-number coordinate is now guaranteed by construction. The
  search/show cross-check test becomes a regression guard rather than the
  guarantee itself.
- `search` collects a `Vec<Record>` per file where it once streamed lines.
  Accepted: transcript files are small and `--failed` / `show` already collect a
  per-file vector; a streaming projection can be reintroduced for `search` alone
  if profiling ever demands it.
- "Block" is now a named domain type (recorded in CONTEXT.md); the previously
  `pub`-but-test-only `parse_line` is removed.
