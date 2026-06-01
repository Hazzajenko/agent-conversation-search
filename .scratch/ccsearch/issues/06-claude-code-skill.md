# The Claude Code skill

Status: ready-for-agent

## What to build

A Claude Code skill that makes `ccsearch` usable from within Claude Code. The skill fires when the user asks Claude to recall or find a past conversation ("did we ever discuss X", "find the session where we set up Y", "what did I decide about Z").

The skill instructs Claude to:

1. Run `ccsearch "<terms>"` for the current Project, or `ccsearch --all` / `ccsearch --project <name>` when the user says "anywhere" or names another project.
2. Read the human-readable output directly (there is no JSON mode by design).
3. When it needs the full conversation, open the matching `.jsonl` path (via `-l`) and read around the Match.
4. Summarize what it found and point the user at the relevant Session.

The skill is thin glue: the binary does the fast mechanical search, Claude does interpretation and follow-up. Document the real flag surface (`--all`, `--project`, `--regex`, `--case-sensitive`, `--thinking`, `--tools`, `--all-content`, `-m`, `-l`, `--claude-dir`).

## Acceptance criteria

- [ ] A skill file exists that triggers on requests to recall/find past conversations
- [ ] The skill documents how to invoke `ccsearch` for current-Project, `--all`, and `--project` searches
- [ ] The skill describes reading the human-readable output and drilling into `.jsonl` files for full context
- [ ] The documented flags match the binary's actual flag surface
- [ ] Verified end-to-end: asking Claude to find a past conversation runs `ccsearch` and returns a useful summary

## Blocked by

- 01-walking-skeleton-current-project-search
- 02-scope-widening-all-and-project
- 05-output-polish
