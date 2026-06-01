# `show --around`: window the Transcript on a turn

Status: ready-for-agent

## What to build

Add turn-addressed windowing to `show`, so that arriving from a search / `--failed` hit lands you at the relevant spot instead of turn 1 of 200.

- Number Messages as **turns** (1-based, in Record order) — the same numbering search emits (issue 09).
- `ccsearch show <id> --around <turn> [--context N]` renders only turns `[turn-N, turn+N]` (default N ≈ 3), with an indicator of how many turns are hidden above and below.
- Bare `show <id>` (no `--around`) still renders the whole Transcript.

## Acceptance criteria

- [ ] Turn numbering matches the numbers search prints for the same Session
- [ ] `--around T --context N` renders only the window, with hidden-turn indicators above/below
- [ ] `--context` overrides the default; bare `show` is unaffected
- [ ] Window bounds clamp correctly at the start and end of a Session
- [ ] Unit plus `assert_cmd` tests for the bounds

## Blocked by

- 07-show-verb-transcript
