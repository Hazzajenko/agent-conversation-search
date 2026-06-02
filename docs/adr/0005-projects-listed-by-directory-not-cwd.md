# `projects` lists by on-disk directory, not by the real cwd

The `projects` verb (issue 17, ADR 0004) lists the Projects in the Store. Each
Session Record carries the real working directory it was recorded in — e.g.
`"cwd":"E:\\projects\\rust\\claude-code-conversation-search"` — so the lister
*could* identify a Project by that real path instead of by the on-disk directory
name. ADR 0001 even sanctions reading the `cwd` field ("we never reverse a
directory name back into a path; when we need the real path we read the `cwd`
field out of the Records"). So why not group Projects by `cwd`?

**Decision:** A Project's **identity is its on-disk directory**, not the real
`cwd`. One directory → one row. The real `cwd` is read only to produce a
human-readable **display label** (the newest Session's `cwd`, falling back to
the encoded directory name). Directories whose names are equal
case-insensitively are folded into one row (ADR 0001's union rule, applied
without a cwd) so the listing behaves identically on case-insensitive
(Windows/macOS) and case-sensitive (Linux/WSL) Stores. A merged row sums its
directories' Session counts and takes the newest last-touched timestamp.

## Considered and rejected

- **Identity = real `cwd` (group Sessions by the path they recorded).** Tempting
  because it is the most "correct" notion of a Project and would both merge
  case-variant directories *and* un-merge lossy-encoding collisions (two real
  paths that encoded to the same directory name). Rejected because it makes
  `projects` rows stop lining up 1:1 with the rest of the tool: `search`,
  `sessions`, `--project`, and `--all` all scope by **directory** (ADR 0001), so
  a cwd-keyed `projects` would list units that `--project <substr>` cannot
  select and that `sessions` groups differently. The verb that tells you "what
  to pass to `--project`" must speak in the same units `--project` consumes. It
  also costs more (every Session's `cwd` must be read and reconciled) and needs
  a policy for Sessions with no readable `cwd`.

- **List raw directories with no case-folding.** Correct on Windows/macOS (where
  case-variant directories cannot coexist), but on Linux/WSL it shows the same
  logical Project as two rows. Rejected: case-folding makes the verb behave the
  same on every OS, and is what `find_project_dirs` already does.

## Consequences

- `projects` rows align 1:1 with the directory-based scoping used everywhere
  else; a displayed Project name (or an alphanumeric chunk of it) can be pasted
  into `--project`.
- The display label can differ from the thing matched: a row shows a real path
  like `E:\projects\rust\…` while `--project` matches the encoded directory
  name. These agree on alphanumeric runs (encoding only rewrites
  non-alphanumerics), so a name-derived substring still selects the Project.
- Case-folding inherits ADR 0001's accepted over-merge risk: on a case-sensitive
  Store, two genuinely distinct paths differing only by case
  (`/home/Foo` vs `/home/foo`) merge into one row. Consistent with ADR 0001 — a
  few extra merged rows are visible and harmless; a missed Project would be a
  silent, confusing failure.
- A lossy-encoding collision (distinct real paths sharing one directory name)
  stays a single row. It cannot be un-merged without making `cwd` the identity,
  which this ADR rejects.
