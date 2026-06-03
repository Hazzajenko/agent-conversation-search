# Grouping failures by marker signature, not exit code or normalised text

> **Superseded by [ADR 0007](0007-structural-signatures-for-non-universal-failures.md).** The marker key proved overfit to one stack, and its largest buckets (`error:`, `(no marker)`) stayed opaque. 0007 keeps the universal markers as stable labels and groups the rest by a bounded structural shape — reversing this ADR's rejection of normalised-line grouping, with reasons.

`--failed` (issue 11) *lists* individual Failures. `stats` (issue 13) *aggregates* them into a counts table to answer "what causes **a lot of** failures." The entire difficulty was always **what makes two Failures similar enough to count together** — ADR 0002 deferred this decision until `--failed` existed and could be pointed at real Failures rather than a whiteboard.

It now has been. Running `--failed --all` against the real Store, the recurring shape is `(tool, exit_code, salient_line)`. But two of those three are noisy as a grouping key, and the third — the marker `salient_line` already matches — is the clean one.

**Decision:** Two Failures are similar when they share the same **`(tool, signature)`**, where the **signature is the highest-priority [`FAILURE_MARKERS`] entry the error text matches** — the very same fixed, priority-ordered lookup table `salient_line` uses to pick the informative line. When no marker matches, the Failure falls into a single per-tool *no-marker* bucket. `salient_line` is refactored to share the marker lookup, so the list and the table can never disagree about a Failure's class.

This keeps `stats` a *fold* over the `Vec<SessionFailures>` `--failed` already produces, so it inherits scope (`--all` / `--project` / `--session`), `--since`, and the optional Query filter for free — exactly as `--failed` did.

## Considered and rejected

- **Put `exit_code` in the key** (`tool × exit_code × …`). Real data kills it from both sides: it *over-splits* the same root cause — pytest `ImportError` exits 2 while pytest `AssertionError` exits 1 — and *under-discriminates* — a real `error[E0433]` and a generic "could not compile … due to N previous errors" are both exit 101. exit_code is a useful *detail to show* in the `--failed` list, not a *key to group on*. Dropped from the key entirely.
- **Group on the normalised salient line** (mask digits / paths / quotes / PIDs / `:line:col`, then compare). Most precise in principle, but the real salient lines are saturated with per-run noise — `due to 2 previous errors`, `(69076)`, `tests::regex_mode_…`, `1 failed, 62 deselected in 0.16s`, the specific quoted arg in a `TypeError`. Making this group well needs an ever-growing masking pass: a fuzzy, "smart" similarity metric — exactly the rabbit hole CONTEXT.md's *predictable-over-clever* principle and the original ask both warn against. Rejected in favour of the marker, which is already a fixed, documented table.
- **Group on `tool × exit_code` alone** (zero normalisation). Maximally predictable but too blunt to be actionable: "PowerShell exit 101: 30" tells you *which* tool hurts, never *what* the error was. The marker restores the "what" while staying deterministic.

## Consequences

`FAILURE_MARKERS` is now load-bearing for *two* features — picking the salient line *and* defining the grouping signature — which raises its importance as the single, documented classifier of failure shape. Marker granularity is deliberately coarse: `error:` lumps compile-rollups, clippy lints, and test-failed messages into one bucket. That is an accepted trade for determinism; a finer signature (e.g. extracting the `E0433` token) can layer on later without changing the key's shape. Failures that match no marker — `<tool_use_error>` wrappers, "File content exceeds maximum", harness-noise like user-rejected calls — collapse into one per-tool no-marker bucket; refining or filtering those is future work, not this decision.
