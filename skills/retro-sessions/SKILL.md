---
name: retro-sessions
description: "Run a retrospective across many past coding agent sessions with agsearch."
disable-model-invocation: true
---

The user has asked for a **retrospective** across past sessions. The text after the command is the **focus**. It is the question you read every session against. An example is "where did agents take too long to find information, or trust out-of-date docs". You suggest changes to the coding agent's **environment** that answer the focus for future runs.

Read every transcript through `agsearch`. Its output is compact and turn-numbered, and each turn number is an **evidence** handle the user can open with `agsearch show <id> --around <turn>`. Run `agsearch --help` or `agsearch help <command>` for flags. If `agsearch --version` fails, stop and tell the user to install it with `cargo install --path .` from a clone of the agsearch repo.

## Steps

1. If the `writing-for-agents` skill is installed, load it. It sets the style for any doc, steering file, or skill text you propose.

2. Fix the **scope** from the focus: how many sessions, which Project, which Harness, and since when. The default is the 10 newest top-level sessions of the current directory's Project, across every Harness. List them with `agsearch sessions` and the scope flags `--since`, `--project`, `--all`, and `--harness`, then take the first N rows. `sessions` also lists the session that runs this skill. Find it with `agsearch current` and leave it out. Done when you hold a numbered list of session ids.

3. **Triage** the scope before you read any transcript, so the deep reads know where to look. Run the commands that bear on the focus:

   - `agsearch usage --since <WHEN>` ranks sessions by tokens. Outliers are where agents struggled.
   - `agsearch --stats --since <WHEN>` and `agsearch --failed` show failed tool calls grouped by error.
   - `agsearch "<phrase>" --tools --since <WHEN>` finds recurring signals such as "No such file", "not found", or a doc name.
   - `agsearch --file <path>` lists every session that read or wrote a file. Use it to test a suspect doc.

   Write down each signal with its session id and turn.

4. **Read** every session in scope against the focus. When you can dispatch subagents, give each session its own subagent on a cheap model, so whole transcripts stay out of your context. Give each one the focus, its session id, and the triage signals for that session, and ask for this return:

   > Run `agsearch show <id>`. Use `--around <turn>` and `--thinking` to look closer. Return each finding that bears on the focus as: turn number, what happened, its cost in turns or failed calls, and the environment gap behind it. Return "none" when the session has nothing on the focus.

   Without subagents, read each session yourself with `agsearch show <id>`. Done when every session id from step 2 has a result, even "none".

5. **Verify** each finding against the repo as it is now. A doc may already be fixed, or a missing pointer may already exist. Drop a finding that the current repo already answers, and say that you dropped it.

6. Group findings that share one gap across sessions. A gap seen in 4 sessions outranks a gap seen in 1. Put each group in one of the categories below, and use "Where fixes live" to choose the file for the fix.

7. Present the groups in order of severity. For each one, give the gap, the proposed change with the file it goes in, and its evidence as `agsearch show <id> --around <turn>` lines. End with a coverage line that gives the count of sessions read and the count that had nothing on the focus.

## Categories

- **Navigation**: the agent took long to find a file or a fact. Fix it with a navigation pointer, or by making a hidden dependency between files visible.
- **Stale docs**: the agent acted on a doc that no longer matched the code. Fix the doc, delete it, or replace the copied fact with a pointer to the command or file that is the source of truth.
- **Automated checks**: the agent made a mistake that a lint, type check, test, or hook could catch. Read the repo's own check commands and CI workflow first. A check that exists but is not wired up is the finding. A repo with no pre-commit hook and no CI check is a finding too.
- **Coding standards**: the reviewer missed a mistake. A mechanical violation, such as a banned API or an import shape, gets a deterministic check. Only a judgement call goes into `CODING_STANDARDS.md`.
- **Steering files**: `CLAUDE.md` or `AGENTS.md` is large, or holds rules that belong in coding standards or a check. Look for no-ops too. A no-op is an instruction that does not change what the agent does.
- **Tool economy**: the agent made expensive tool calls, or a custom CLI or MCP tool returned more tokens than the agent needed. `agsearch usage <id>` shows the cost of each call.
- **Information access**: the agent did not have a fact it needed, such as dev server logs or read access to a third-party service.

## Where fixes live

Work goes through implementation, then review. The implementation agent has the most context pressure, because it explores, writes code, and debugs. The review agent gets a diff and has the least. Put coding standards on the review side.

- `CLAUDE.md` and `AGENTS.md` are in the context of every agent in the repo. Keep them short, and use them mostly for navigation pointers.
- `CODING_STANDARDS.md` is read during review only.
- Docs are reference files that other files point to. Look for an existing doc before you write a new one.
- Skills hold reference that an agent reaches through the skill description, or commands that the user runs by name.
