# 0015. Release via release-plz, cargo-dist, and crates.io Trusted Publishing

Date: 2026-09-14

## Status

Accepted

## Context

The repo is going public and needs to ship `agsearch` two ways: as a crate on
crates.io (`cargo install agsearch`) and as prebuilt binaries for people who
do not have a Rust toolchain. Releases must not depend on a maintainer's
machine, and the repo must not hold a long-lived crates.io token.

Three shapes were considered:

1. **Tag by hand.** Push `vX.Y.Z`; one workflow builds binaries and runs
   `cargo publish`. Simple, but the version bump and changelog are manual and
   easy to forget, and the crates.io token must live as a repository secret.
2. **cargo-dist alone.** Generates a binary-building workflow and installers
   from a tag, but it does not bump versions, write a changelog, or publish
   the crate.
3. **release-plz plus cargo-dist.** release-plz reads conventional commits,
   opens a release PR that bumps `Cargo.toml` and `CHANGELOG.md`, and on merge
   tags and publishes the crate. cargo-dist reacts to that tag and builds the
   binaries and the GitHub Release.

## Decision

Option 3.

- **release-plz** owns the version, the changelog, the tag, and `cargo publish`.
  Its GitHub Release creation is switched off so the two tools do not fight
  over the same release.
- **cargo-dist** owns the binaries, checksums, shell and PowerShell installers,
  and the GitHub Release body. It triggers on the tag release-plz pushes.
- **Trusted Publishing** (OIDC via `rust-lang/crates-io-auth-action`) replaces a
  stored `CARGO_REGISTRY_TOKEN`. It needs one manual first publish so the crate
  exists and the GitHub repo can be linked on crates.io.
- release-plz runs with a fine-grained personal access token rather than the
  default `GITHUB_TOKEN`, because tags and PRs made by `GITHUB_TOKEN` do not
  trigger other workflows, which would leave the tag without binaries.

## Consequences

- Commits must keep the conventional-commit prefixes (`feat:`, `fix:`, ...);
  they drive the version bump and changelog text.
- Every merge to `main` may open or refresh a release PR. Merging that PR is the
  release act; nothing else is needed.
- Two generated files are owned by tools and should not be hand-edited:
  `.github/workflows/release.yml` (regenerate with `dist generate`) and the
  release sections of `CHANGELOG.md`.
- The crate name `agsearch` is now fixed; crates.io does not allow renames.
- Rotating the maintainer PAT is the one recurring manual chore.
