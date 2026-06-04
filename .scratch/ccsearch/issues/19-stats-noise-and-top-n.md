# `--stats` long tail: stdout mistaken for errors, and no top-N

Status: needs-triage
Category: enhancement

## Why

`--stats` is the "what's been breaking?" view. On a single Project it produces
the tidy table the README shows. Across `--all` (10 projects here) it degrades
to ~150 rows, the great majority count-1 singletons — the aggregate stops
aggregating, and the signal (`Edit: has not been read ×44`, `Read: does not
exist ×41`) is buried under noise.

Two distinct causes, observed on the real Store:

1. **stdout treated as an error signature.** When a Bash command exits non-zero
   but only printed benign output — section headers from compound diagnostic
   commands (`echo "=== user settings ==="`), test-progress dots, `git grep | wc
   -l` probes that legitimately return non-zero — `salient_line`'s
   last-non-empty-line fallback (lib.rs:1200) grabs the *header*, not an error.
   So `=== user settings ===`, `---README---`, `--- AUDIT.md self-ref (expected)
   ---` each become their own singleton signature. The marker table deliberately
   buckets harness noise (rejected/unavailable/cancelled) but has no notion of
   "command exited non-zero yet succeeded at its purpose."

2. **No top-N / min-count threshold.** Even with perfect signatures, a
   whole-Store stats table wants a way to collapse the long tail (e.g. a
   `… +N more signatures (count 1)` summary line, or `-m`/`--top N`).

## Agent Brief

**Category:** enhancement

**Summary:** Reduce `--stats` noise so the aggregate stays useful at `--all`
scale. Two independent levers; either helps, both is best.

**Desired behavior (sketch — needs design before build):**
- A way to bound the table: a `--top N` cap, or fold all count-1 (and tool-less)
  rows into a single `… +N more singleton signatures` trailer. Decide which in
  triage; ADR 0003 owns the stats output contract, so any change is an
  amendment/superseding ADR.
- Optionally, recognise "benign non-zero" stdout shapes so a section-header line
  never becomes a signature. Risky — easy to over-filter a real error. Treat as
  a separate, lower-priority sub-task; the top-N lever is the safer win.

**Acceptance criteria (to be firmed up in triage):**
- [ ] `--stats --all` on the real Store is readable without scrolling past a wall
      of singletons.
- [ ] No real, recurring error signature is hidden by the new collapsing.
- [ ] ADR 0003 amended/superseded to record the output-shape change.

**Out of scope:**
- Reworking the marker table's universal-vs-structural split (ADR 0003 / 0007 —
  that part works well).

## Comments

### 2026-06-04 — Filed

Found while running `--stats --all` to "analyse patterns" across the whole
Store. The number/path normalisation and the universal-vs-structural design are
genuinely good; the failure is purely at `--all` scale + the benign-non-zero
stdout case. Left `needs-triage` because the output-shape decision (top-N vs
fold-singletons vs both) wants a maintainer call and an ADR amendment.
