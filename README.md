# Arshy

**AI Agent's native shell.** Structured output, auto-mode intelligence, security sandbox -- all through MCP.

## Why Arshy?

When an AI Agent runs a shell command via raw Bash, it gets unstructured text. Arshy provides structured events with file:line locations, error codes, and fix hints.

| | Raw Bash | Arshy |
|-|----------|-------|
| Output | Unstructured text | Structured events (error/warning/file/line) |
| Error location | Agent searches text | Precise file:line:col + source context |
| Long tasks | Block or timeout | Async + real-time notifications + graceful kill |
| Security | None | Command filter + path sandbox + permission levels + audit |
| History | None | JSONL cross-session query |

## Quick Start

### Install

**One-Click Installer (macOS & Linux):**
```bash
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh
```
This script automatically detects your platform, compiles/downloads the binaries, configures transparent shell hooks, and registers background services under launchd (macOS) or systemd (Linux).

**Or install from source manually:**
```bash
git clone https://github.com/iZoy/Arshy.git && cd arshy
cargo build --release
cp target/release/arshy target/release/arshyd ~/.local/bin/
```

Requires: Rust 1.75+, macOS or Linux. SQLite is bundled.

### Connect to Claude Code

```bash
arshy install
```

This registers arshy as an MCP server in `~/.claude.json` and adds tool permissions to `~/.claude/settings.json`. Restart Claude Code to activate.

### First Command

```bash
arshy run "echo hello world"
```

Short commands (ls, git status, echo) return instantly as plain text. Long commands (cargo build, npm test) run async with structured output. No configuration needed -- `mode:auto` detects the right path automatically.

## Features

- **2-tool MCP model** -- `arshy_exec` (execute/manage tasks) + `arshy_query` (query events). ~60% fewer tokens than separate tools.
- **Auto-mode intelligence** -- short commands get zero-overhead raw output; long commands get structured events with async notifications.
- **37 built-in parsers** -- tsc, cargo, jest, eslint, go, python, docker, kubectl, terraform, and more.
- **Security sandbox** -- command filter, path sandbox, permission levels, audit log.
- **Terminal UI** -- `arshy run "cargo build" --format pretty` for structured visualization.
- **`arshy doctor`** -- diagnose MCP integration issues with a single command.
- **`arshy analyze`** -- impact analysis report across recent tasks.
- **Custom parsers** -- TOML (declarative regex, stateless or stateful patterns) for community contributions.

## Configuration

```toml
# ~/.config/arshy/config.toml
[daemon]
max_task_duration_ms = 3600000
max_concurrent_tasks = 4

[security]
access_level = "full"
blocked_patterns = ["rm -rf /"]
```

See [Configuration](docs/guides/configuration.md) for all options.

## Documentation

- [Getting Started](docs/getting-started/install.md)
- [Product Philosophy & Token Restraint](docs/explanation/philosophy.md)
- [Configuration](docs/guides/configuration.md)
- [Custom Parsers](docs/guides/custom-parser-toml.md)
- [Architecture](docs/explanation/architecture.md)

## License

MIT
