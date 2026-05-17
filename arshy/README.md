# Arshy

Structured shell execution for Claude Code. Replaces raw Bash with typed, queryable command execution — smart sync, audit log, command filtering, and task lifecycle management.

## MCP Tools

| Tool | Description |
|------|-------------|
| `arshy_exec` | Execute shell commands with action dispatch: run, cd, kill, list, tail, subscribe |
| `arshy_query` | Query structured events from completed/running tasks (compile errors, lint warnings) |

## Setup

Run `/arshy-setup` to install or diagnose. Requires no configuration.

## How It Works

- **Short commands** (ls, git status, echo): return instantly like native Bash
- **Long commands** (cargo build, pytest): run async with structured output and task lifecycle
- **Subscribe**: block until a long task finishes — no polling
- **Daemon**: runs on-demand, auto-starts/auto-exits to save resources

## Uninstall

Run `/arshy-setup uninstall` or manually:
```bash
rm -f ~/.cargo/bin/arshyd ~/.cargo/bin/arshy
pkill arshyd
```
