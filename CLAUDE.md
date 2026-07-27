# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Shell Execution (DOG FOODING)

arshy is our own product. Route ALL shell commands through `arshy_exec` — this is both validation and quality assurance. No additional agent configuration is needed beyond installing the plugin.

**Why:** Every command exercised through arshy is a real-world test. Using Bash to build a shell replacement is self-defeating.

**Fallback:** If `arshy_exec` returns `DaemonUnreachable`, run `scripts/restart.sh`. Only if that fails, use Bash as last resort.

**Commit Policy:**
- Run `cargo test` through `arshy_exec` before every commit
- Never commit if arshy_exec is unavailable (fix the daemon first)

## Build & Test Commands

```
cargo build                        # debug build (both binaries)
cargo build --release              # release build
cargo build --bin arshy            # proxy/CLI only
cargo build --bin arshyd           # daemon only

cargo test                         # all tests
cargo test --lib --bin arshy --bin arshyd   # unit tests only (CI command)
cargo test --test integration      # integration tests
cargo test --bin arshyd fixture_cargo       # single parser fixture

ARSHY_BLESS=1 cargo test --bin arshyd       # bless parser fixture snapshots

cargo fmt --all -- --check         # format check
cargo clippy --all-targets -- -D warnings   # lint (zero warnings enforced)
cargo doc --no-deps --document-private-items # doc build check
```

## CLI Commands

```bash
arshy run "cmd"                    # execute command (pretty format auto-detected)
arshy run "cmd" --format pretty    # terminal UI output
arshy run "cmd" --format json      # raw JSON output
arshy benchmark                    # run parser benchmark across 37 parsers
arshy run "cmd" --errors-only      # only return error-level events
arshy parser reload                # hot-reload parsers + show diff
arshy parser list                  # list loaded parsers
```

## Architecture

Two-binary Rust crate (~22,000 lines) implementing an MCP server that replaces raw Bash for AI agents.

### Two-Binary Design

- **`arshy`** (proxy/CLI): Receives MCP JSON-RPC from stdin, connects to daemon via UDS, forwards tool calls, returns structured results to stdout. Also serves as CLI (`arshy run "echo hello"`). Can auto-start the daemon.
- **`arshyd`** (daemon): Background process — command execution (PTY), output parsing, SQLite storage, event streaming, security enforcement.

Communication: JSON-RPC 2.0 over Unix Domain Socket, JSON Lines framing.

### Core Data Flow

1. Agent calls `arshy_exec` MCP tool -> proxy translates to IPC request
2. Daemon executor spawns command via PTY
3. Output flows through 6-layer parser pipeline: JSON -> Stateful patterns -> TOML regex -> Crash detection -> Heuristic error filter -> Raw fallback
4. Events deduplicated (consecutive identical lines collapsed), stored in SQLite, published via EventBus
5. Error events enriched with source context (±3 lines) and git change correlation
6. Results returned as structured MCP response

### Key Modules

| Module | Path | Purpose |
|--------|------|---------|
| Proxy | `src/proxy/mod.rs` | MCP stdio <-> UDS bridge (~1430 lines) |
| Executor | `src/daemon/exec/mod.rs` | Task scheduling, auto-mode intelligence (~1620 lines) |
| IPC Handler | `src/daemon/ipc_handler.rs` | JSON-RPC dispatch, notification routing (~1344 lines) |
| Parser Engine | `src/daemon/parser/mod.rs` | 6-layer pipeline orchestration (~715 lines) |
| Parsers | `src/daemon/parser/*.rs` | toml.rs, stateful.rs, crash.rs, heuristic.rs, dedup.rs, json.rs, detect.rs |
| Context | `src/daemon/context/` | Source context enrichment + git correlation |
| Store | `src/daemon/store/` | SQLite CRUD, prune, version cache |
| Config | `src/config/` | Loading priority: CLI > env > file > defaults |
| Security | `src/daemon/security/` | Command filter, path sandbox, audit log |
| IPC | `src/ipc/` | JSON-RPC types, UDS transport |
| MCP | `src/mcp/` | Tool definitions, protocol types |

### Auto-Mode Intelligence

The executor distinguishes short vs long commands:
- **Short** (ls, echo, git status): zero-overhead, raw stdout returned instantly
- **Long** (cargo build, npm test): structured path with parsed events, 60s sync timeout before async degradation

`is_short_command()` checks word count, char length, known build/test prefixes, and inspection tool allowlist.

## Parser System

37 builtin TOML parser definitions in `parsers/builtin/`. Each parser has fixture tests in `parsers/builtin/tests/<tool>/` with `.txt` input and `.json` expected output (42 fixtures total).

### Error Code Hints

7 error codes across 4 languages in `parsers/errors/*.toml`. When a parser extracts an error code, the executor automatically attaches `hint: {cause, fix}` to the event. The `HintDb` is loaded at startup via `include_str!`. Only codes with non-obvious fixes are included (common codes like E0308, TS2345 are excluded since LLMs already know them).

### Adding a Parser

1. Create `parsers/builtin/<tool>.toml` with `[meta]` and `[[pattern]]` sections
2. Create `parsers/builtin/tests/<tool>/` with `.txt` input and `.json` expected output
3. Run `ARSHY_BLESS=1 cargo test --bin arshyd` to auto-generate expected JSON
4. Verify fixture tests pass with >=95% field accuracy (type, severity, code, file, line)
5. Mark deprecated patterns with `deprecated = true` and `replaced_by = "new-pattern-name"`

## Project Conventions

### Commit Style
- `feat:` new feature | `fix:` bug fix | `docs:` documentation | `refactor:` | `test:` adding tests

### Code Quality (enforced in CI)
- `rustfmt.toml`: max_width=100, 4 spaces, Unix newlines
- `clippy.toml`: allow-unwrap-in-tests=true
- CI runs: fmt check, clippy -D warnings, unit tests, doc build — all must pass

### Pattern Lifecycle
- Never remove a pattern without a deprecation cycle
- Add `since_version` to new patterns
- If changing the TOML schema, bump `schema_version` in affected parser files
