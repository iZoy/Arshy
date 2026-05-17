# Arshy Plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Package arshy as a Claude Code plugin with pre-compiled binary distribution, on-demand daemon lifecycle (5min idle exit), and a setup/diagnostic skill.

**Architecture:** Four independent work streams — plugin skeleton (static files), daemon idle exit (proxy code ~30 lines), setup skill (SKILL.md), and CI/CD release workflow (GitHub Actions). The idle exit is the only code change; everything else is packaging and automation.

**Tech Stack:** Rust (existing codebase), GitHub Actions, Bash (install script snippet in SKILL.md)

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `arshy/README.md` | Create | User-facing plugin documentation |
| `arshy/.claude-plugin/plugin.json` | Create | Plugin metadata (name, version, author, description) |
| `arshy/.mcp.json` | Create | MCP server config pointing to `~/.cargo/bin/arshy --from-mcp` |
| `skills/arshy-setup/SKILL.md` | Create | Setup skill: binary detection, platform-aware download, checksum, install |
| `.github/workflows/release.yml` | Create | CI/CD: build on tag push, upload binaries to GitHub Releases |
| `src/proxy/mod.rs` | Modify | Add idle timeout to main select! loop; call `daemon/shutdown` on idle |
| `src/mcp/instructions.rs` | Modify | Commit static copy of tool schema for plugin distribution |

---

### Task 1: Plugin Skeleton

**Files:**
- Create: `arshy/.claude-plugin/plugin.json`
- Create: `arshy/.mcp.json`

**Purpose:** The minimum files Claude Code needs to discover and load the arshy MCP server. No code logic.

- [ ] **Step 1: Create plugin README**

Create `arshy/README.md`:

```markdown
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
```

- [ ] **Step 2: Create `arshy/.claude-plugin/plugin.json`**

```json
{
  "name": "arshy",
  "version": "0.1.0",
  "description": "Structured shell execution for Claude Code — smart sync, audit log, command filtering, and task lifecycle management. Prefer arshy_exec over raw Bash for ALL shell commands.",
  "author": {
    "name": "izoy"
  }
}
```

- [ ] **Step 2: Create `arshy/.mcp.json`**

```json
{
  "mcpServers": {
    "arshy": {
      "type": "stdio",
      "command": "${HOME}/.cargo/bin/arshy",
      "args": ["--from-mcp"],
      "env": {}
    }
  }
}
```

- [ ] **Step 4: Commit**

```bash
git add arshy/README.md arshy/.claude-plugin/plugin.json arshy/.mcp.json
git commit -m "feat: add arshy plugin skeleton (README + plugin.json + .mcp.json)"
```

---

### Task 2: Daemon Idle Exit

**Files:**
- Modify: `src/proxy/mod.rs:109-190` (main select! loop)
- Modify: `src/proxy/mod.rs:62-69` (variable declarations above loop)

**Purpose:** When the MCP proxy receives no tool calls for 5 minutes, send `daemon/shutdown` to the daemon. The proxy keeps running. Next tool call triggers `connect_or_start()` which auto-restarts the daemon. Default idle timeout 300s, overridable via `ARSHY_IDLE_TIMEOUT_SECS` env var.

- [ ] **Step 1: Add idle timeout constant and tracking variable**

In `src/proxy/mod.rs`, after the `let mut request_tasks` line (around L69), add:

```rust
// Idle timeout: shut down daemon after 5 min of inactivity.
// Override with ARSHY_IDLE_TIMEOUT_SECS.
let idle_timeout_secs: u64 = std::env::var("ARSHY_IDLE_TIMEOUT_SECS")
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(300);
let idle_timeout = tokio::time::Duration::from_secs(idle_timeout_secs);
let mut last_activity = tokio::time::Instant::now();
```

- [ ] **Step 2: Update `last_activity` after each tool call**

Inside the `"tools/call"` branch of the main match (around L145), after `handle_tool_call(...)` completes (both success and error paths), add:

```rust
// After the existing handle_tool_call block completes,
// update idle timestamp for both success and error paths.
last_activity = tokio::time::Instant::now();
```

Insert this after the `"tools/call"` match arm ends — right before the next `"notifications/cancelled"` arm.

- [ ] **Step 3: Add idle timeout branch to the main select!**

In the main `tokio::select!` at line 110, add a third branch after the notification branch:

```rust
tokio::select! {
    // ── Stdin branch ────────────────────────────────────────────
    result = stdin.read_line(&mut line) => {
        // ... existing code unchanged ...
    }
    // ── Notification branch ─────────────────────────────────────
    notif = notif_fut => {
        // ... existing code unchanged ...
    }
    // ── Idle timeout branch (NEW) ───────────────────────────────
    _ = tokio::time::sleep_until(last_activity + idle_timeout) => {
        tracing::info!(
            "daemon idle for {}s, sending shutdown",
            idle_timeout_secs
        );
        // Fire-and-forget: if daemon is already down, send_request fails silently
        let _ = daemon.send_request(ipc::METHOD_SHUTDOWN, serde_json::json!({})).await;
        // Don't break — proxy stays alive for next request.
        // connect_or_start() will revive the daemon when needed.
        continue;
    }
}
```

- [ ] **Step 4: Reset `last_activity` on reconnect**

In the reconnect success path (around L156-L159), after `daemon = new_conn; notif_rx = new_notif_rx;`, add:

```rust
last_activity = tokio::time::Instant::now();
```

- [ ] **Step 5: Build and verify compilation**

```bash
cargo build 2>&1
```
Expected: `Finished` with no errors.

- [ ] **Step 6: Run existing tests**

```bash
cargo test 2>&1
```
Expected: all 255 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/proxy/mod.rs
git commit -m "feat: daemon idle exit after 5min inactivity (ARSHY_IDLE_TIMEOUT_SECS)"
```

---

### Task 3: arshy-setup Skill

**Files:**
- Create: `skills/arshy-setup/SKILL.md`

**Purpose:** A user-invoked skill (`/arshy-setup`) that detects the platform, downloads the correct pre-compiled arshy binary from GitHub Releases, verifies the SHA256 checksum, and installs to `~/.cargo/bin/`. Also serves as a diagnostic when MCP connection fails.

- [ ] **Step 1: Create `skills/arshy-setup/SKILL.md`**

```markdown
---
name: arshy-setup
description: Install, update, or diagnose the arshy Claude Code plugin. Use when the user says "setup arshy", "install arshy", "configure arshy", "arshy not working", "arshy not found", "update arshy", or when arshy_exec returns DaemonUnreachable.
argument-hint: [install|update|diagnose|uninstall]
allowed-tools: [Bash, Read, Write]
---

# Arshy Setup

## Overview

Installs or diagnoses the arshy structured shell execution plugin. Arshy provides `arshy_exec` (smart-sync shell commands) and `arshy_query` (structured event queries) as Claude Code MCP tools.

## Arguments

$ARGUMENTS — if empty, default to "install".

## Instructions

### install

1. **Detect platform:**

```bash
OS=$(uname -s)
ARCH=$(uname -m)
case "$OS-$ARCH" in
  Darwin-arm64) TARGET="aarch64-apple-darwin" ;;
  Darwin-x86_64) TARGET="x86_64-apple-darwin" ;;
  Linux-x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
  Linux-aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
  *) echo "Unsupported platform: $OS-$ARCH"; exit 1 ;;
esac
echo "Detected platform: $TARGET"
```

2. **Check if already installed:**

```bash
if [ -x "$HOME/.cargo/bin/arshyd" ] && [ -x "$HOME/.cargo/bin/arshy" ]; then
  echo "arshy already installed:"
  $HOME/.cargo/bin/arshy --version 2>&1 || true
  echo "Use 'update' to upgrade, 'diagnose' to check health."
  exit 0
fi
```

3. **Download latest release:**

Get the latest version tag from GitHub API, then download:

```bash
INSTALL_DIR="${ARSHY_INSTALL_DIR:-$HOME/.cargo/bin}"
mkdir -p "$INSTALL_DIR"

VERSION="0.1.0"  # updated on each release
URL="https://github.com/izoy/arshy/releases/download/v${VERSION}/arshy-v${VERSION}-${TARGET}.tar.gz"
CHECKSUM_URL="${URL}.sha256"

echo "Downloading arshy v${VERSION} for ${TARGET}..."
curl -fsSL "$URL" -o /tmp/arshy.tar.gz
curl -fsSL "$CHECKSUM_URL" -o /tmp/arshy.tar.gz.sha256
```

4. **Verify checksum:**

```bash
EXPECTED=$(cat /tmp/arshy.tar.gz.sha256 | awk '{print $1}')
ACTUAL=$(shasum -a 256 /tmp/arshy.tar.gz | awk '{print $1}')
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "Checksum mismatch! Expected: $EXPECTED, got: $ACTUAL"
  echo "Aborting install. Try again or download manually from:"
  echo "  https://github.com/izoy/arshy/releases"
  rm /tmp/arshy.tar.gz /tmp/arshy.tar.gz.sha256
  exit 1
fi
echo "Checksum verified OK"
```

5. **Extract and install:**

```bash
tar -xzf /tmp/arshy.tar.gz -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR/arshyd" "$INSTALL_DIR/arshy"
rm /tmp/arshy.tar.gz /tmp/arshy.tar.gz.sha256
echo "arshy installed to $INSTALL_DIR"
```

6. **Verify installation:**

```bash
"$INSTALL_DIR/arshy" status 2>&1
```

If it prints daemon status (running or not), installation is successful. The daemon will auto-start on the first MCP tool call.

### update

Same as install, but skip the "already installed" check. Overwrites existing binaries.

### diagnose

1. Check if binaries exist and are executable
2. Check daemon process status: `pgrep arshyd`
3. Check socket: `ls -la ~/.local/share/arshy/arshyd.sock`
4. Try a health check: `$HOME/.cargo/bin/arshy status`
5. Report findings with suggested fixes

### uninstall

```bash
rm -f "$HOME/.cargo/bin/arshyd" "$HOME/.cargo/bin/arshy"
# Also kill running daemon
pkill arshyd 2>/dev/null || true
echo "arshy uninstalled"
```
```

- [ ] **Step 2: Commit**

```bash
git add skills/arshy-setup/SKILL.md
git commit -m "feat: add arshy-setup skill (install/update/diagnose/uninstall)"
```

---

### Task 4: GitHub Actions Release Workflow

**Files:**
- Create: `.github/workflows/release.yml`

**Purpose:** On version tag push (e.g., `v0.1.0`), build binaries for macOS (arm64 + x86_64) and Linux (x86_64), generate SHA256 checksums, create a GitHub Release, and upload artifacts.

- [ ] **Step 1: Create `.github/workflows/release.yml`**

```yaml
name: Release

on:
  push:
    tags:
      - 'v[0-9]+.[0-9]+.[0-9]+'

permissions:
  contents: write

jobs:
  build:
    strategy:
      matrix:
        include:
          - target: aarch64-apple-darwin
            os: macos-latest
          - target: x86_64-apple-darwin
            os: macos-latest
          - target: x86_64-unknown-linux-gnu
            os: ubuntu-latest

    runs-on: ${{ matrix.os }}

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Build release binary
        run: cargo build --release --target ${{ matrix.target }}

      - name: Package artifacts
        run: |
          VERSION=${GITHUB_REF#refs/tags/}
          ARCHIVE=arshy-${VERSION}-${{ matrix.target }}.tar.gz
          cd target/${{ matrix.target }}/release
          tar -czf ../../../${ARCHIVE} arshyd arshy
          cd ../../..
          shasum -a 256 ${ARCHIVE} > ${ARCHIVE}.sha256

      - name: Upload to release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            arshy-*.tar.gz
            arshy-*.tar.gz.sha256
          generate_release_notes: true
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add GitHub Actions release workflow (multi-platform binary builds)"
```

---

### Task 5: Static Tool Schema (Plugin Distribution)

**Files:**
- Modify: `src/mcp/instructions.rs` (or create a generated file)

**Purpose:** The plugin's `.mcp.json` only configures the server command. The actual tool schema (JSON definitions for `arshy_exec` + `arshy_query`) and MCP instructions are served by the proxy during `initialize`. No changes needed — this task verifies the existing code serves correctly and documents the static schema for reference.

- [ ] **Step 1: Verify `tools/list` response is correct**

```bash
cargo test mcp::tests::tool_definitions 2>&1
```
If no such test exists, add a quick verification:
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | nc -U ~/.local/share/arshy/arshyd.sock 2>/dev/null | python3 -m json.tool | head -40
```
Expected: both `arshy_exec` (with `subscribe` in action enum) and `arshy_query` are listed.

- [ ] **Step 2: Document — no code changes needed**

The proxy already serves `tool_definitions()` and `default_instructions()` from `src/mcp/instructions.rs` during MCP initialization. The plugin distributes these dynamically, so no static copy is needed in the plugin directory.

---

### Task 6: End-to-End Verification

**Files:** None (manual verification only)

**Purpose:** Confirm the full plugin lifecycle works: install → MCP tools available → idle exit → auto-restart.

- [ ] **Step 1: Verify plugin discovery**

Place the plugin directory in the plugins cache and confirm Claude Code loads it.

- [ ] **Step 2: Verify idle exit**

```bash
# Start proxy with short timeout for testing
ARSHY_IDLE_TIMEOUT_SECS=10 arshy --from-mcp &
PROXY_PID=$!

# Make a tool call
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | nc -U ~/.local/share/arshy/arshyd.sock

# Wait 10 seconds, then check daemon exited
sleep 12
pgrep arshyd && echo "FAIL: daemon still running" || echo "PASS: daemon exited"

# Make another call — daemon should auto-restart
echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | nc -U ~/.local/share/arshy/arshyd.sock
pgrep arshyd && echo "PASS: daemon restarted" || echo "FAIL: daemon not restarted"

kill $PROXY_PID 2>/dev/null
```

- [ ] **Step 3: Verify subscribe still works end-to-end**

```bash
SOCK=~/.local/share/arshy/arshyd.sock

# Start async task
echo '{"jsonrpc":"2.0","id":1,"method":"task/run","params":{"command":"sleep 2 && echo ok","mode":"async"}}' | nc -U "$SOCK"
# Copy task_id from output, then:
echo '{"jsonrpc":"2.0","id":2,"method":"task/subscribe","params":{"task_id":"<task_id>"}}' | nc -U "$SOCK"
# Expected: blocks ~2s, returns {"exit_code":0,"status":"completed"}
```

- [ ] **Step 4: Run full test suite**

```bash
cargo test 2>&1
```
Expected: all tests pass.

---

### Task 7: Final Commit and Tag

- [ ] **Step 1: Push to GitHub**

```bash
git push origin main
```

- [ ] **Step 2: Create release tag**

```bash
git tag -a v0.1.0 -m "arshy v0.1.0 — Claude Code plugin release"
git push origin v0.1.0
```

Expected: GitHub Actions triggers the release workflow, builds binaries, uploads to Releases.
```

