# Arshy — Your Shell for Claude Code

Arshy takes over shell execution in Claude Code. Install the plugin, restart, and arshy becomes your primary shell — no CLAUDE.md edits, no per-project configuration, no manual mode selection.

## Why

Bash gives you raw text. Arshy gives you **structured output**:
- Compiler errors become typed `diagnostic` events with `file`, `line`, `code`
- Test results become `test_result` events with pass/fail counts  
- Stack traces are auto-detected across Go, Python, Rust, Node.js, and shell
- 37 built-in parsers: cargo, tsc, eslint, python, go, npm, webpack, and more

## How It Takes Over

Arshy claims shell ownership through **four independent layers**:

| Layer | Mechanism | What It Does |
|-------|-----------|--------------|
| **MCP Protocol** | `experimental.preferredShell` capability + authoritative `instructions` | Tells the agent "I am your shell — do not use Bash" |
| **Plugin Hook** | `PreToolUse` on Bash tool | Intercepts raw Bash calls and redirects to arshy_exec |
| **Shell Router Skill** | Auto-triggered `/shell-router` | Quick reference and usage guidance |
| **Setup Skill** | `/arshy-setup` | One-command install, update, and diagnostics |

## MCP Tools

| Tool | Description |
|------|-------------|
| `arshy_exec` | Your primary shell. action: run, cd, kill, list, tail, subscribe |
| `arshy_query` | Query structured events (diagnostics, test results, crashes) |

## Setup

```bash
claude plugin install arshy@arshy-marketplace
# Then run once:
/arshy-setup
```

No other configuration needed. No CLAUDE.md to edit.

## How It Works

- **Short commands** (ls, git status, echo): return text instantly like native Bash
- **Long commands** (cargo build, pytest): run async with structured output — you never think about sync vs async
- **Subscribe**: block until a long task finishes — no polling
- **Daemon**: runs on-demand, auto-starts when needed, exits after 5 min idle

## Skills

| Skill | Purpose |
|-------|---------|
| `/shell-router` | Route all shell commands through arshy_exec |
| `/arshy-setup` | Install, update, or diagnose the arshy daemon |

## Uninstall

```bash
claude plugin uninstall arshy
rm -f ~/.local/bin/arshyd ~/.local/bin/arshy
```
