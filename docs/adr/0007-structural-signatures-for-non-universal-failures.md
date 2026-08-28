# Grouping failures by universal markers plus structural shape

**Supersedes [ADR 0003](0003-grouping-failures-by-marker-signature.md).**

ADR 0003 made `(tool, marker-signature)` the grouping key for `stats`, where the signature is the highest-priority [`FAILURE_MARKERS`] entry the error text matches, and everything unmatched falls into one per-tool `(no marker)` bucket. It explicitly **rejected** grouping on a normalised salient line ("mask digits / paths / quotes … then compare") as a fuzzy, ever-growing masking pass — the *predictable-over-clever* rabbit hole.

Running it against the real Store exposed two problems 0003 named but accepted:

1. **The biggest buckets are the least useful.** `error:` lumps every compile-rollup, clippy lint, and `test failed` line together (`19 ✗ PowerShell error:` in this project); `(no marker)` is the catch-all residue and is routinely the single largest row. Both answer "a lot of failures" without answering *which*.
2. **The marker table is overfit to one stack.** `error[`, `panicked at`, `Traceback`, `assertion …`, `fatal:` are Rust/Python/git syntax. For any other user these match nothing, so their real errors all collapse into `(no marker)`. The signature silently stops working off the author's machine — the table is a *personal* classifier wearing a universal one's clothes.

**Decision:** Split `FAILURE_MARKERS` into two kinds, and key `stats` on a *hybrid* signature.

- **Universal markers** (`fm`) — Claude Code's own tool/harness vocabulary (`has not been read`, `String to replace not found`, `exceeds maximum allowed tokens`, `rejected`, `cancelled`, `unavailable`, …) and OS-level shell errors (`No such file`, `command not found`, `does not exist`, `unexpected EOF`). These strings are emitted by the harness or the OS, identical for every user, so their `label` stays a **stable, deterministic signature**. Where the identifying phrase sits amid variable text (a filename, a command name), a fixed label also beats structure.
- **Display hints** (`hint`) — programming-language error syntax (`error[`, `panicked at`, `Traceback`, `Error:`/`error:`, `fatal:`, `assertion`). Still used by `salient_line` to highlight the right line, but they **no longer name a bucket**. Their Failures — and the old `(no marker)` residue — group by `structural_signature` of the salient line instead.

`structural_signature` is a **pure two-rule normalisation**: a whitespace token containing `/` or `\` becomes `<path>`; each run of ASCII digits becomes a single `N`; the result is truncated to 60 chars. `(no marker)` survives only for a Failure with no error text at all.

```
error: could not compile `agsearch` … due to 2 previous errors  →  error: could not compile `agsearch` … due to N prev…
thread 'main' panicked at src/lib.rs:5:9:                        →  thread 'main' panicked at <path>
error[E0433]: cannot find type                                  →  error[EN]: cannot find type
```

## Why this is not the option 0003 rejected

0003 weighed *marker* versus *normalisation* as whole-table alternatives. This is neither:

- **It is a hybrid.** The cross-user-meaningful buckets (harness + tool + OS) keep deterministic labels. Structure replaces only the residue, where 0003's own alternative was the useless `(no marker)` lump — which 0003 itself called "future work."
- **It is a fixed pure function, not a similarity metric.** Two masking rules, no thresholds, no fuzzy comparison, no growth. `structural_signature` is as deterministic as a marker lookup; it just normalises before keying. That is *predictable*, even if it is more than the marker.

The real-data evidence: the `error:` lump decomposed into the actionable shape — `could not compile` (8), `test failed --lib` (4), `test failed --test cli` (3) — and the digit/path masking absorbed the worst per-run noise 0003 feared (PIDs, `:line:col`, `due to N previous errors`).

## Considered and rejected

- **More masking rules** (strip backticked spans, `tests::…` paths, quoted args) to fix the remaining fragmentation. This is the exact rabbit hole 0003 warned of, and the line we will not cross: the value of a *pure two-rule* normalisation is that it is bounded and predictable. A third rule is a smell that the marker was the right tool for that case — add a **universal marker**, not a masking rule.
- **Reverting to 0003.** Keeps determinism but leaves the two real defects: the most-populous buckets stay opaque, and the signature stays silently overfit to one stack. The hybrid keeps 0003's determinism where it mattered (universal vocabulary) and pays it only where 0003 had already conceded `(no marker)`.

## Consequences

- **The signature type changes** from `Option<&'static str>` to `String` (`FailureGroup.signature`, `failure_signature`, `group_failures`). The `(no marker)` label is now emitted by `failure_signature`, not by `format_stats`.
- **`salient_line` is unchanged** — it still consults every marker, hint included, so the `--failed` list highlights the same lines as before. Display and grouping still read the same line; the table just masks it.
- **Known cost — accepted:** the table is longer, with a tail of count-1 rows where a salient line carries an unmasked identifier (a test name, a quoted type). This is the fragmentation 0003 predicted; the guardrail above (no third masking rule) is what keeps it bounded rather than chased.
- **Accuracy of recurrence improves where it counts:** the buckets that used to hide the most (`error:`, `(no marker)`) now split into legible shapes, and the signature now means the same thing on anyone's machine.
