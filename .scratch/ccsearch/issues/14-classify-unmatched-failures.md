# Classify the `(no marker)` failures: grow the signature table, decide on harness noise

Status: needs-triage

## Why

`--stats` (issue 13) groups Failures by `(tool, matched FAILURE_MARKERS entry)`, falling back to a per-tool `(no marker)` bucket. Run against the real Store, **the three biggest groups are all `(no marker)`** (Bash 101, Edit 63, PowerShell 59 — roughly half of all Failures). So `stats` answers "what causes a lot of failures?" with a shrug for its largest buckets. `FAILURE_MARKERS` was built (issue 11) to pick a salient *display line*, where a last-line fallback is fine; as a *grouping signature* (ADR 0003) it under-classifies. ADR 0003 already named this future work — this is that work.

## What's actually in the catch-all (real-Store counts, 2026-06-02)

Two distinct populations, and they want different treatment:

**A. Legitimate tool Failures that simply match no marker** — these should be *classified*:
- `<tool_use_error>` wrapper — **189** occurrences; wraps the real message and currently defeats marker matching.
- `String to replace not found` (Edit) — **39**
- `has not been read yet` (Edit/Write/Read order-of-operations) — **~98** (count inflated by echoes; real but fewer)
- `exceeds maximum allowed tokens` (Read) — **21**
- `is not recognized` (PowerShell unknown cmdlet) — **7**
- `Blocked:` (harness command guard) — **9**

**B. "Failures" that arguably aren't a tool erroring at all** — these need a *decision*, not a marker:
- `tool use was rejected` / `doesn't want to proceed` (the user declined the call) — **~28**
- `temporarily unavailable` (model/classifier down, retryable infra) — **19**

## The design fork (why this is `needs-triage`, not `ready-for-agent`)

Population B is the open question. A `tool_result` with `is_error:true` for a *user rejection* or a *transient infra outage* fits the CONTEXT.md definition of **Failure** structurally, but not in spirit — the tool didn't error, the run was declined or the backend blinked. Options:

1. **Classify them** like everything else (add `rejected` / `unavailable` markers). Simple, consistent, but pollutes "what causes failures" with things that aren't code problems.
2. **Filter them out** of failure analysis entirely (both `--failed` and `--stats`). Cleaner signal, but `--failed` silently drops records that *are* `is_error:true` — needs to be a documented, predictable rule, and may warrant updating the **Failure** glossary entry in CONTEXT.md (and possibly an ADR, since it narrows a domain term).
3. **Separate bucket** — count them but under an explicit `rejected` / `unavailable` signature so they're visible but not conflated with real errors.

Decide this first; it changes the CONTEXT.md definition of Failure either way.

## Likely acceptance criteria (to firm up at triage)

- [ ] Strip `<tool_use_error>…</tool_use_error>` wrappers before signature/salient-line matching (a pure step, shared by `salient_line` and `failure_signature` so they stay consistent). Noted as pending since issue 11.
- [ ] Add population-A content markers to `FAILURE_MARKERS` (or a sibling table), in priority order, with a unit test per shape from the five real samples above. Keep it a fixed, documented lookup table — no fuzzy matching (the ADR 0002 / ADR 0003 principle).
- [ ] Resolve population B per the chosen fork option; update the **Failure** glossary in CONTEXT.md accordingly, and an ADR if the decision narrows the term.
- [ ] Re-run `--stats --all`; the `(no marker)` buckets should no longer dominate the top of the table (the regression check is the real Store, not a fixture).

## Out of scope

- No fuzzy/normalised similarity — still grouping by a fixed signature table, just a bigger/cleaner one. This issue grows the classifier; it does not change the grouping *mechanism* decided in ADR 0003.

## Blocked by

- 13-stats-aggregate (done)
