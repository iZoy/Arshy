# Changelog

All notable changes to the arshy project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- Chain-aware parser detection: `&&`, `||`, `;` chained commands now match parser by segment
- Short path for 40+ read-only inspection tools (grep, find, cat, git, ls, etc.)
- Parser quality infrastructure: bless mode (`ARSHY_BLESS=1`), per-field precision/recall
- TOML schema v1.0: `schema_version`, `since_version`, `deprecated`, `replaced_by`
- ReDoS safety validator: rejects nested quantifiers and overlapping alternation
- Parser change audit: diff summary logged on reload/hot-reload
- Config versioning: `version` field with forward-compatibility warning
- Telemetry counters: atomic task/connection/event metrics in stats and health endpoints
- Daemon connection rate limiting: `Semaphore(64)` on accept loop
- Health check exponential backoff (1s → 60s) on consecutive failures
- Sandbox mode validation: unimplemented modes rejected with clear error
- Integration tests: 3 e2e tests spawning real daemon + validating health/run/parser
- CI workflow: `cargo fmt --check`, `cargo clippy`, `cargo test`, `cargo doc` on PRs
- Standalone compilers (rustc, gcc, clang, g++, clang++) added to long-output detection

### Fixed
- No-output failed commands now report `[command failed: exit code N]` instead of empty text
- `unwrap()` in `CommandFilter::from_config` replaced with `Result` propagation
- Spawn-lock file cleaned up on spawn failure path
- All MCP error responses now include `data.retryable` field for agent decision-making
- MCP tool descriptions clarified (cwd absolute path, parse_hint accepts parser names)

### Changed
- `TomlParser` cached in `ParserSession` instead of cloned per-line
- `CRASH_LOG.lock().unwrap()` replaced with `.expect("CRASH_LOG poisoned")`
- Deprecated patterns with replacements are skipped at session creation time

## [0.1.0] — 2026-05-17

### Added
- Initial release: arshy MCP server with stdio proxy + Unix domain socket daemon
- 20 builtin TOML parsers for common CLI tools
- 2-tool MCP model: `arshy_exec` (run/kill/list/tail/cd/subscribe) + `arshy_query`
- Smart auto-mode: short commands sync, long commands async with structured output
- Rhai scripting engine for stateful cross-line parsers
- Crash parser: cross-language traceback detection (Go, Python, Rust, Node, shell)
- Raw fallback with severity classification
- Parser hot-reload via filesystem watcher
- Security: command filtering, audit logging, sandbox path restrictions
- SQLite-backed task store with event persistence
- Idle daemon exit after 5 minutes (configurable via `ARSHY_IDLE_TIMEOUT_SECS`)
