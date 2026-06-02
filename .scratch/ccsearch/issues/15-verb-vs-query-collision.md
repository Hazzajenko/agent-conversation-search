# A bare Query of `search` or `show` is swallowed by the subcommand

Status: needs-triage
Category: bug

## Why

`search` and `show` are both clap subcommands *and* plausible Query text. Because the subcommand wins, a user who runs `ccsearch search` gets `ccsearch: a query is required …` instead of a search for the literal word "search", and `ccsearch show` tries to resolve a Session named (nothing) rather than searching for "show". Any single-token Query that collides with a verb name is unreachable in the bare form.

This is correct clap behaviour, not a crash, and the error message is clear — so it's a low-severity sharp edge, not a breakage. Filing it so the trade-off is a decision rather than an accident.

## Observed (2026-06-02)

```
$ ccsearch search
ccsearch: a query is required (or use `ccsearch show <session>`, or `--failed`)   # exit 1
```

(A multi-word Query that merely *starts* with `search`, e.g. `ccsearch "search index"`, is unaffected — clap only treats the bare token as a verb.)

## Possible directions (to decide at triage)

1. **Document only** — accept it; note in `--help`/README that to search for a verb word you write `ccsearch search <something>` won't work and suggest `-e '^search$'` via the explicit `search` verb (`ccsearch search search`). Cheapest; the collision stays.
2. **Disambiguate on the error path** — when the bare invocation resolves to a verb with no further args (the "query required" branch), and the token is itself a valid Query, hint: "did you mean to search for \"search\"? use `ccsearch search search`". Keeps clap's parse, improves the dead-end.
3. **`--` / explicit-verb guidance** — confirm `ccsearch search show` and `ccsearch search search` actually search for those literals, and surface that in help as the escape hatch. (Likely already works via the explicit `search` verb — verify and document rather than build.)

Leaning option 3 + a doc line: the explicit verb is the existing escape hatch; the fix may be purely making it discoverable.

## Out of scope

- Renaming or removing the `search`/`show` verbs.
- Any change to how multi-token queries parse — only the bare single-verb-token case is in question.

## Comments
