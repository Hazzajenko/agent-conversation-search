# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |
| _(none)_                   | `done`               | Implemented and verified; terminal state |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

`done` is a local addition — the five mattpocock state roles have no terminal "completed" state, so finished work would otherwise linger as `ready-for-agent` and pollute "what's ready to pick up?" queries. Transition: `ready-for-agent` / `ready-for-human` → `done` once the work is merged and verified. Like `wontfix`, it is terminal. On GitHub, `done` means closing the issue (`gh issue close <n> --comment "..."` summarising what shipped) rather than applying a label; the label form only applies to the legacy `.scratch/` markdown issues.

Edit the right-hand column to match whatever vocabulary you actually use.
