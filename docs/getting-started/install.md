# Installation

## Prerequisites

- **Rust 1.75+** ([rustup.rs](https://rustup.rs))
- **macOS or Linux**
- SQLite is bundled -- no system dependency needed.

## From Source

```bash
git clone https://github.com/iZoy/Arshy.git
cd arshy
cargo build --release
```

This produces two binaries:

| Binary | Purpose |
|--------|---------|
| `target/release/arshy` | CLI client + MCP proxy |
| `target/release/arshyd` | Background daemon |

### Install to PATH

```bash
cp target/release/arshy target/release/arshyd ~/.local/bin/
# or ~/.cargo/bin/ if it is in your PATH
```

Verify the installation:

```bash
arshy --version
arshyd --version
```

## Register with Claude Code

```bash
arshy install
```

This does three things:

1. Registers arshy as an MCP server in `~/.claude.json`
2. Adds tool permissions (`arshy_exec`, `arshy_query`, `Bash(arshy *)`) to `~/.claude/settings.json`
3. Starts the daemon if it is not already running

**Restart Claude Code** after installing for the MCP tools to appear.

## First Command

```bash
arshy run "echo hello world"
```

Short commands return plain text instantly. Long commands run async with structured events. The `mode:auto` setting (default) detects the right path automatically.

## Verify Your Setup

```bash
arshy doctor
```

This checks binaries, daemon status, MCP registration, and permissions, reporting pass/fail for each item.

## Auto-Start on Login

To have the daemon start automatically:

```bash
# macOS
arshy install-launchd

# Linux (systemd)
arshy install-systemd
```

## Troubleshooting

### Daemon not starting

Check the logs:

```bash
# macOS
cat ~/Library/LaunchAgents/com.arshy.daemon.err

# Linux
journalctl --user -u arshyd
```

Start manually to see errors directly:

```bash
arshyd --log-level debug
```

### MCP tools not appearing in Claude Code

1. Run `arshy doctor` to diagnose
2. Run `arshy install` to re-register
3. Restart Claude Code (full quit, not just close window)
4. Check `~/.claude.json` has an `arshy` entry under `mcpServers`

### Socket connection errors

The daemon communicates over a Unix domain socket. If the socket file is stale:

```bash
arshy daemon restart
```

If that does not help, remove the stale socket and restart:

```bash
rm -f ~/.local/share/arshy/arshyd.sock
arshy daemon start
```

### "arshy: command not found"

Ensure `~/.local/bin` (or wherever you installed) is in your `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Add this to your shell profile (`~/.zshrc`, `~/.bashrc`) to persist it.

### Build fails with Rust version error

Upgrade Rust to the latest stable:

```bash
rustup update stable
```

Arshy requires Rust 1.75 or later.

## File Locations

| Path | Purpose |
|------|---------|
| `~/.config/arshy/config.toml` | Configuration file |
| `~/.local/share/arshy/arshyd.sock` | Unix socket (daemon IPC) |
| `~/.local/share/arshy/arshy.db` | SQLite database (WAL mode) |
| `~/.local/share/arshy/arshyd.pid` | Daemon PID file |
| `~/.arshy/parsers/` | User custom parsers (TOML / Rhai) |
| `~/.claude.json` | Claude Code MCP server config |
| `~/.claude/settings.json` | Claude Code permissions |
