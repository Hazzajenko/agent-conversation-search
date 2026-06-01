# (Deferred) `stats`: aggregate failure counts

Status: needs-triage

## What to build

The aggregate counterpart to `--failed`: instead of listing individual Failures, emit a **counts table** — "Bash: 23 failures; 7× `cargo test` (exit 2), 4× `grep` (exit 1)…" — to answer "what causes *a lot of* failures."

**Deliberately deferred** (ADR 0002). The hard part is grouping *similar* failures (the "a lot of *like*" in the original ask): exact-command grouping is near-useless because every command differs; error-signature grouping (tool + exit code + salient line) is better but is its own rabbit hole. Don't design this on a whiteboard — ship `--failed` first (issue 11), stare at real Failures, *then* decide what "similar" means and whether this warrants its own ADR.

## Acceptance criteria

- [ ] (To be defined after living with `--failed`.)

## Blocked by

- 11-failed-tool-calls
