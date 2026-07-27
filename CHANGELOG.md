# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Removed
- **Web dashboard** (`arshy analyze --web`): removed the browser-view feature; terminal `pretty`/`json` formats remain for humans verifying agent output.
- **Rhai scripting engine**: removed the `rhai` crate dependency and `.rhai` user-script parser path. The TOML-native stateful pattern matcher (`stateful.rs`, formerly `rhai.rs`'s Patterns variant) is retained; `ParserType::Rhai` renamed to `ParserType::Stateful`. Rationale: zero production parsers used the script engine; the capability is revivable from git history when user demand emerges.
- **`task/stdin` IPC method**: removed the permanent "not supported yet" stub (unreachable from the agent MCP surface).
- Docs: `guides/custom-parser-rhai.md` and `reference/parser-rhai-api.md` deleted; parser spec's Rhai section removed.

### Changed
- **Adaptive run-response event inlining**: a `run` response no longer inlines up to 200 events. On failure it inlines up to 20 error-severity events; on success up to 5 warning/info events. When truncated, the response carries `events_truncated: true` and an `events_hint` directing the agent to `arshy_query` for the rest. Preserves the one-round-trip flow (validated by dogfooding) while honoring token restraint.

### Fixed
- Documentation drift: parser pipeline relabeled `Stateful(Rhai)` → `Stateful(模式状态机)`; `long_about` parser count 20 → 37; CLAUDE.md line count 14,500 → 22,000; ROADMAP hook-interception and v0.2.0 release status marked complete.

## [0.2.0] - 2026-07-07

### Added
- **Security & Sandbox Boundaries**:
  - Peer UID verification for Unix Domain Socket IPC. The daemon now rejects connections from any local process with a mismatched UID (`libc::getuid()`).
  - Path constraint sandbox mode (`--sandbox_mode workspace`) limiting command execution strictly to directories within the active workspace root.
- **Agent Hook Interception & Wrapper**:
  - Support for Claude Code's native `PreToolUse` hook interceptor in `~/.claude/settings.json`.
  - Built-in `claude-hook` protocol parser executing robust POSIX single-quote escaping for shell inputs and preventing recursion loops.
  - Strict command exit code propagation ensuring failures in background shell tasks correctly trigger non-zero shell exit codes for agents.
- **Google Antigravity Core Integration**:
  - Auto-installation/uninstallation of `arshy` MCP server registration in `~/.gemini/config/mcp_config.json`.
  - Parent process recognition in the shell wrapper for `"antigravity"`, `"gemini"`, and `"agy"`.
- **Telemetry & Dirty-Mark Flushing**:
  - Added `agent_delivered_bytes` to trace the exact amount of structured log payloads transferred to the agent's context.
  - Optimized database flush loop using a dirty-state caching pattern writing to SQLite every 1 second.

### Fixed
- Cleaned up all compiler warnings and resolved unused variables and dead-code blocks (`total_structured_events_bytes`, `compute_token_efficiency`) across the engine.
