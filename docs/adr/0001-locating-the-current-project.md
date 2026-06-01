# Locating the current Project's directory

Claude Code stores each Project under `~/.claude/projects/<encoded-cwd>/`, where the encoded name is the working directory with **every non-alphanumeric character replaced by `-`**. This encoding is lossy (`a\b\c` and `a-b-c` and `a.b.c` all collapse to `a-b-c`) and the drive-letter case is non-deterministic on Windows — the same `E:` drive appears as both `E--…` and `e--…` depending on how the shell reported the cwd when the session started.

**Decision:** To find the current Project, we forward-encode `cwd` (non-alphanumeric → `-`) and match it **case-insensitively** against the directory names in `~/.claude/projects/`. If several directories match, we search the **union** of them. We never attempt to reverse a directory name back into a path; when we need the real path we read the `cwd` field out of the Records themselves.

**Why:** Forward encoding is deterministic; reverse decoding is impossible. Case-insensitive matching absorbs the drive-letter wobble (and Windows filesystems are case-insensitive anyway). Unioning on collision is safer than guessing — a missed Session is a silent, confusing failure, whereas a few extra results are visible and harmless.
