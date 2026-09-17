# Changelog

All notable changes are documented here. Arshy follows Semantic Versioning.

## [Unreleased]

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

## [0.1.0-alpha.1] - Unreleased

First public preview for macOS and Linux. The preview validates standard MCP
onboarding and real-task demand; its API may change before 0.1.0.
