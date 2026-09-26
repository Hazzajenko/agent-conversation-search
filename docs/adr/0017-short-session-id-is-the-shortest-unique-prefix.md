# The short session-id is the shortest unique prefix

ADR 0002 made the short session-id the first 8 characters of the `sessionId`. ADR 0010 said UUIDs make collisions a non-concern. OpenCode ids are not UUIDs. Every id starts with `ses_`, and the next characters come from the creation time. The first 8 characters of an OpenCode id are the same for all Sessions made in a period of hours, so a fixed 8 characters does not identify a Session.

**Decision:** For every Harness, the short session-id is the shortest prefix that is unique across all Stores, with a minimum of 8 characters. For display, the `ses_` prefix of an OpenCode id is removed. Resolution accepts a prefix with or without `ses_`.

## Considered and rejected

- **The first 8 characters after `ses_`.** These are still time-ordered, so Sessions made close together collide.
- **The full id.** It is correct but too long to copy and type, which is the purpose of the short id.
- **A different rule per Harness.** A short id that looks the same but acts differently per Harness is surprising.

## Consequences

- Before the tool prints a short id, it must know the ids of all Sessions in all Stores.
- A short id can grow longer after a new Session is added. An old short id can then become ambiguous. `show` already reports an ambiguous prefix, so the user sees this.
- Claude Code and Codex short ids stay 8 characters in almost all cases.
