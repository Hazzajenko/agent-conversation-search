# Project is the logical working directory, derived per Harness

The tool now searches two Harnesses (CONTEXT.md). Claude Code materialises
Projects as directories under its Store, and ADR 0005 made that directory the
Project's identity. Codex has no per-project directories at all — its Store is a
date-partitioned tree of rollout files, and the only project evidence is the
`cwd` field in each Session's `session_meta` Record. Some notion of Project must
cover both, and the same repo used with both Harnesses should be one Project.

**Decision:** A Project's identity is its **logical working directory**,
represented by the encoded directory name (every path separator replaced with
`-`, ADR 0001's rule). Each Harness derives that key its own way: Claude Code
Sessions already live in a directory named by the encoding; a Codex Session's
`cwd` is read from `session_meta` and passed through the same encoding. Equal
keys (case-insensitively, per ADR 0005's folding) are one Project, regardless of
Harness. `--project` continues to substring-match the encoded key, so the units
`projects` lists remain exactly the units `--project` selects.

## Considered and rejected

- **Identity = raw real `cwd`.** ADR 0005 rejected this for Claude Code because
  `search`, `sessions`, and `--project` all scope by encoded directory name; a
  cwd-keyed model would list units the flags cannot select. That objection
  stands. Encoding the Codex `cwd` into the *same* key space gets the merge
  behaviour a cwd identity promised without breaking the 1:1 alignment.
- **Separate Project spaces per Harness.** Simplest to build, but the user's
  question is "what happened in this repo," not "what happened in this repo per
  tool" — two rows for one directory is the tool failing to know they are the
  same place. Rejected.
- **Drop the `projects` verb for Codex.** Rejected: full-parity is the goal, and
  the `cwd` field makes derivation cheap (one line read per file).

## Consequences

- ADR 0005 is amended, not superseded: directory identity stays for Claude Code;
  the encoded name is promoted from "how Claude names directories" to the
  cross-Harness Project key.
- Codex Project derivation requires reading each Session's `session_meta` line —
  the first line of the file, so enumeration stays cheap.
- The encoding is lossy (ADR 0005's collision caveat) and now also merges a
  Claude and a Codex Session whose distinct real paths encode identically.
  Accepted for the same reason as before: over-merge is visible and harmless.
- A Codex Session with no readable `cwd` belongs to no Project: invisible to
  `--project` scoping but still reachable via `--all` and by `sessionId`.
