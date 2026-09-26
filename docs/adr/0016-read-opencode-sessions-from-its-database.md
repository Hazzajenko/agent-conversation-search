# Read OpenCode Sessions from its SQLite database

OpenCode 1.2 and later keeps every Session in one SQLite file, `opencode.db`, in its data directory. A Session is a `session` row. Its content is `message` rows, and each message owns `part` rows with a JSON `data` column. OpenCode writes to the file in WAL mode while it runs. The file can be several gigabytes. ADR 0010 assumed a Harness adapter parses a Session file. This Harness has no Session file.

**Decision:** The OpenCode adapter opens `opencode.db` read-only with the bundled `rusqlite` crate. It does not use the `immutable` flag, so reads see writes that OpenCode has committed to the WAL. One `message` row with its parts maps to one Record. `text`, `reasoning`, and `tool` parts map to Blocks. Other part kinds are noise. The adapter can filter with SQL before it builds Records, for example a `LIKE` prefilter on `part.data` for search. The matching rules stay in the shared projections.

## Considered and rejected

- **Copy the database before reading.** A copy of a 3 GB file for each command is too slow.
- **Run the `sqlite3` CLI as a subprocess.** It adds a runtime dependency that most machines do not have.
- **Read the pre-1.2 JSON tree too.** OpenCode migrates it into the database and leaves the old files in place. Reading both would show each old Session twice. Upstream issue #13654 reports that the migration can skip for some upgrades. Those users can run OpenCode once to fix it.
- **One `part` row per Record.** Turn numbering in `show` and search would then count parts, not Messages, and differ from the other Harnesses.

## Consequences

- The build now compiles SQLite from C source. The release pipeline in ADR 0015 must still build every target.
- Store discovery checks for `opencode.db`. A data directory without it is a missing Store.
- Enumeration is one query, not a directory walk. Project, Title, and parent come from `session` columns.
- A Session that OpenCode writes during a command can show a partial last turn. This is the same as for a `.jsonl` file that a Harness appends to.
