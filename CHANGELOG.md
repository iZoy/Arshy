# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
