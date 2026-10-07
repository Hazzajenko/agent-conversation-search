# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
## [0.2.0](https://github.com/Hazzajenko/agent-conversation-search/compare/v0.1.1...v0.2.0)

### Added
- add retro-sessions skill for multi-session retrospectives
- count opencode file reads and writes as touches ([#60](https://github.com/Hazzajenko/agent-conversation-search/pull/60))
- search and show opencode sessions ([#58](https://github.com/Hazzajenko/agent-conversation-search/pull/58))
- list opencode sessions from its database ([#57](https://github.com/Hazzajenko/agent-conversation-search/pull/57))
- show the shortest unique prefix as the short session-id ([#56](https://github.com/Hazzajenko/agent-conversation-search/pull/56))

### Fixed
- *(skill)* match retro-sessions to agsearch scope and turn rules

### Other
- *(deps)* bump clap in the cargo group across 1 directory ([#51](https://github.com/Hazzajenko/agent-conversation-search/pull/51))
- review PRs with the OpenCode review agents ([#71](https://github.com/Hazzajenko/agent-conversation-search/pull/71))
- list opencode write tools under --written ([#60](https://github.com/Hazzajenko/agent-conversation-search/pull/60))
- assert exact opencode --stats counts and signatures ([#59](https://github.com/Hazzajenko/agent-conversation-search/pull/59))
- say --failed and --stats cover opencode sessions ([#59](https://github.com/Hazzajenko/agent-conversation-search/pull/59))
- cover opencode failures in --failed and --stats ([#59](https://github.com/Hazzajenko/agent-conversation-search/pull/59))
- replace the session file path with a session locator ([#55](https://github.com/Hazzajenko/agent-conversation-search/pull/55))
- replace dashes, semicolons, and asides in README and CONTEXT ([#52](https://github.com/Hazzajenko/agent-conversation-search/pull/52))
## [0.1.1](https://github.com/Hazzajenko/agent-conversation-search/compare/v0.1.0...v0.1.1)

### Other
- drop the date from changelog release headings
- allow Dependabot edits to the cargo-dist workflow
- pin MSRV via toolchain input so Dependabot does not bump it

## [0.1.0]

Initial public release.
