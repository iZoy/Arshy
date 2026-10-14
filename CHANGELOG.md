# Changelog

All notable changes are documented here. Arshy follows Semantic Versioning.

## [Unreleased]

### Changed

- Linux release artifacts target `x86_64-unknown-linux-musl` and
  `aarch64-unknown-linux-musl` (static) so installers run on Ubuntu 22.04,
  Debian 12, and other older glibc hosts. macOS targets are unchanged.
  `install/install.sh` matches the new archive names.

### Notes

- A historical annotated tag `v0.1.0` exists on the remote and points at the
  2026-08-04 initial commit. It has no GitHub Release and is not a published
  crate. Before the first public prerelease, maintainers should delete or
  rename that tag so it does not collide with a future stable `v0.1.0`.
  Suggested commands (run only after you confirm): see
  [docs/how-to/publish-release.md](docs/how-to/publish-release.md).

## [0.1.0-alpha.1] - 2026-10-14

First public preview for macOS and Linux. The preview validates standard MCP
onboarding and real-task demand; its API may change before 0.1.0.

### Changed

- MCP tool results now use standard structuredContent with a text fallback.
- Every execution result exposes status and exit_code.
- The representative failure event is named primary_diagnostic.
- Process output is read in bounded byte chunks and reports invalid UTF-8 or
  truncated lines without stopping the stream.
- Doctor supports a minimal local JSON report.
- Daemon start and restart wait for a responsive status endpoint before
  reporting readiness.
- Dogfood checks now validate the alpha doctor JSON and generated onboarding
  Prompt references.
- Release automation waits for all supported targets before creating a draft.

### Documentation

- Prepared the public-preview installation, demo, known-issues, security,
  feedback, and operating guides.

### Fixed

- Cargo's trailing `error: could not compile ...` and
  `warning: ... generated N warnings` lines are now summary events instead of
  extra diagnostics, so a single compiler error is reported as one error.
- The stderr error heuristic no longer escalates parser-classified summary
  events to errors.
- `arshy daemon start` waits up to 10s (matching the MCP proxy) before
  reporting that the daemon did not start, tolerating slow first starts.
- Declared the minimum supported Rust version (`rust-version = "1.87"`).
