# Roadmap

## v0.1.0-dev.1 — internal release candidate

- generic MCP stdio server with a deliberately thin surface;
- structured parser pipeline, source locations, codes, context, and dedup;
- verified multi-platform installer and GitHub/crates.io release artifacts;
- quality-v1 opt-in component analytics with reproducible counters;
- cwd directory guard, command filter, peer checks, and audit log;
- CI quality gates, integration tests, and coverage thresholds.

## v0.2.x

- improve parser coverage from real anonymized fixtures;
- publish longitudinal quality-v1 datasets with corpus and platform metadata;
- improve Windows/WSL packaging only after a maintainer commits to support;
- evaluate a Homebrew formula after install telemetry is stable.

## Explicit non-goals

Agent-specific adapters, shell hooks, automatic project-file mutation, token
savings promises, and OS/container sandboxing are outside the public contract.
