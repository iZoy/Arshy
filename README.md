# Arshy

**AI Agent's native shell.** Structured output, auto-mode intelligence, security sandbox — all through MCP.

```
Agent loads Skill → knows which CLI to use → executes command → arshy (shell) → structured output
```

## Why Arshy?

When an AI Agent runs a shell command via raw Bash, it gets unstructured text and must parse it itself. Arshy sits between the Agent and the shell:

| | Raw Bash | Arshy |
|-|----------|-------|
| Output | Unstructured text | Structured events (error/warning/file/line) |
| Error location | Agent searches text | Precise `file:line:col` + source context |
| Long tasks | Block or timeout | Async + real-time notifications + graceful kill |
| Security | None | Command filter + path sandbox + permission levels + audit |
| History | None | SQLite cross-session query |
| Short commands | Direct | Same — zero overhead |

## Quick Start

### Install

```bash
# From source
git clone https://github.com/iZoy/Arshy.git
cd arshy
cargo build --release
cp target/release/arshy target/release/arshyd ~/.local/bin/
```

Requires: Rust 1.75+, macOS or Linux. SQLite is bundled.

### Connect to Claude Code

```bash
arshy install
```

This registers arshy as an MCP server in `~/.claude/settings.json`. Restart Claude Code to activate.

### First Command

```bash
arshy daemon start
arshy run "echo hello world"
```

Short commands (ls, git status, echo) return instantly as plain text. Long commands (cargo build, npm test) run async with structured output.

## Key Features

### 2-Tool MCP Model

Agents see just 2 tools instead of 5+ separate ones:

| Tool | Purpose |
|------|---------|
| `arshy_exec` | Execute commands, manage tasks (run/kill/list/tail) |
| `arshy_query` | Query structured events from a task |

~60% fewer tool definition tokens. Agent doesn't need to think "which tool should I use?"

### Auto-Mode Intelligence

`mode:auto` (default) automatically detects:

- **Short commands** (≤5 words, no pipes/flags) → zero-overhead path: skip store, skip parser, return raw text
- **Long commands** → async + structured events + real-time notifications

No configuration needed. The agent just calls `arshy_exec` with `action:"run"` for everything.

### 37 Built-in Parsers

tsc, cargo, jest, vite, eslint, go, python, cc, npm, webpack, prettier, swc, esbuild, clippy, make, gradle, cargo-test, mocha, pip, pnpm, terraform, kubectl, helm, aws, docker, uv, ruff, turbo, nx, deno, bun, biome, oxlint, vitest, git, curl, ssh — each extracts structured events (errors, warnings, file locations) from tool output.

Plus: **generic JSON parser** (auto-detects `--json` output), **crash parser** (Go/Python/Rust/Node/Shell tracebacks), and **stderr error detection** for any command.

### Custom Parsers

Two tiers for community contribution:

- **TOML** — declarative regex matching, no code required
- **Rhai** — scriptable stateful parsing for complex output (docker, kubectl, terraform)

### Security Sandbox

- **Command filter** — whitelist/blacklist regex patterns
- **Path sandbox** — restrict working directory to project paths
- **Permission levels** — `read-only` (query only) vs `full` (execute)
- **Audit log** — all commands logged independently

### Daemon Lifecycle

```bash
arshy daemon start/stop/restart   # process management
arshy stats                       # execution metrics
arshy install-launchd             # macOS auto-start
arshy install-systemd             # Linux auto-start
```

### Terminal UI

Pretty-print command results with structured visualization:

```bash
arshy run "cargo build" --format pretty
```

```
╭──────────────────────────────────────────────────────────╮
│ ✗ Failed   3 errors, 0 warnings  exit 1   370ms         │
├──────────────────────────────────────────────────────────┤
│ Root cause: mismatched types                              │
├──────────────────────────────────────────────────────────┤
│ ✗ Diagnostic  E0308                                       │
│   mismatched types                                        │
│   src/app.ts:42:10                                        │
│                                                           │
╰──────────────────────────────────────────────────────────╯
```

Formats: `--format pretty` (terminal UI), `--format json` (raw JSON), `--format auto` (default: TTY detection).

### Parser Benchmark

Run `arshy benchmark` to measure parser performance across all 37 builtin parsers:

```
Scope: 43 fixtures across 37 builtin parsers
  Information Density:  3.5 actionable fields/event (structured) vs 0 (raw text)
  Token Efficiency:     1.4x compression (up to 12x for webpack)
  Error Location Speed: 24% of fixtures — structured faster
  Parser Accuracy:      100% (43/43 fixtures)
  Feature Value:        85% events with file:line, 60% with error codes
```

### Parser Management

```bash
arshy parser reload   # hot-reload from disk + show diff
arshy parser list     # list loaded parsers
```

## Architecture

```
┌──────────────────────────────────────────────────────┐
│  MCP Client (Claude Code / Cursor / any MCP host)    │
│    stdin/stdout JSON-RPC 2.0                          │
└──────────────┬───────────────────┬───────────────────┘
               │                   │
          ┌────▼────┐        ┌─────▼─────┐
          │  arshy   │◄──────►│  arshyd   │
          │ (proxy)  │  UDS   │ (daemon)  │
          └──────────┘        └─────┬─────┘
                                    │
                    ┌───────────────┼───────────────┐
                    │               │               │
               ┌────▼────┐   ┌─────▼─────┐   ┌────▼────┐
               │ Executor │   │  Parser   │   │  Store  │
               │ (PTY)    │   │  Engine   │   │(SQLite) │
               └──────────┘   └───────────┘   └─────────┘
                                    │
                         ┌──────────┼──────────┐
                         │          │          │
                     ┌───▼──┐  ┌───▼──┐  ┌───▼──┐
                     │ TOML │  │ Rhai │  │ JSON │
                     │  20  │  │ 脚本  │  │ 通用  │
                     └──────┘  └──────┘  └──────┘
```

Two binaries:
- **`arshy`** — CLI client + MCP proxy (stdin/stdout ↔ UDS)
- **`arshyd`** — daemon (PTY execution, parsing, storage, notifications)

## Configuration

Config at `~/.config/arshy/config.toml`:

```toml
[daemon]
max_task_duration_ms = 3600000
max_concurrent_tasks = 4

[parser]
hot_reload = true
dirs = ["~/.arshy/parsers"]

[security]
access_level = "full"
blocked_patterns = ["rm -rf /", "curl.*\\|.*sh"]
```

See [Configuration Reference](docs/reference/config-schema.md) for all fields.

## MCP Prompts

Arshy ships with 3 prompt templates:

- `analyze_build_failure` — diagnose why a build failed
- `diagnose_test_failure` — identify failing tests and suggest fixes
- `review_task_output` — summarize structured command output

## Documentation

- [Getting Started](docs/getting-started/install.md) — install + first command
- [Configuration](docs/guides/configuration.md) — config format, env vars, layering
- [Custom Parsers](docs/guides/custom-parser-toml.md) — write your own parser
- [Security](docs/guides/security.md) — command filtering, sandboxing, audit
- [Architecture](docs/explanation/architecture.md) — component deep dive
- [Parser Specification v1.0](docs/spec/parser-spec.md) — open spec for parser definitions
- [Full Reference](docs/index.md) — all docs

## License

MIT
