# Folding singleton rows in the stats table

**Amends [ADR 0003](0003-grouping-failures-by-marker-signature.md) / [ADR 0007](0007-structural-signatures-for-non-universal-failures.md)** (the stats output contract).

ADR 0007 accepted a known cost: a tail of count-1 rows wherever a salient line carries an unmasked identifier. On a single Project that tail is short. At `--all` scale it dominates — ~150 rows on the real Store, the great majority singletons — and the aggregate stops aggregating: the signal (`Edit: has not been read ×44`) is buried under one-off rows, many of them benign stdout (section headers from compound commands) that `salient_line`'s last-line fallback mistook for errors (issue #19).

**Decision:** `format_stats` folds every count-1 row into a single trailer line:

```
… +N more singleton signatures
```

- Rows with count ≥ 2 render exactly as before, biggest first.
- The trailer appears only when N > 0.
- **Exception:** when *every* row is a singleton, the table renders unfolded — an all-singleton table has no noise burying signal, and folding it would hide everything behind a bare trailer.

Grouping (`group_failures`) is untouched; this is purely a rendering change in `format_stats`.

## Considered and rejected

- **A `--top N` flag.** Opt-in means the default stays unreadable at `--all` scale, and it caps by rank rather than by the recurrence property that actually separates signal from noise. Rejected in triage (maintainer, 2026-08-27).
- **Recognising "benign non-zero" stdout shapes** so section headers never become signatures. Attacks the cause rather than the symptom, but is easy to over-filter — a real error can look like arbitrary stdout. Deferred; may be filed separately if folding proves insufficient.
- **A min-count threshold above 1.** Count-2+ recurrence is already the meaningful line; a higher threshold hides genuinely repeating failures.

## Consequences

- `--stats --all` on the real Store reads as the short recurring-signature table plus one trailer, instead of a wall of one-offs.
- A genuinely new, once-seen failure is no longer visible in the whole-Store table until it recurs (or unless the scope is narrow enough that all rows are singletons). `--failed` still lists every Failure individually — stats is the recurrence view.
