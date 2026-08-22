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
if [ -x "$HOME/.local/bin/arshyd" ] && [ -x "$HOME/.local/bin/arshy" ]; then
  echo "arshy already installed:"
  $HOME/.local/bin/arshy --version 2>&1 || true
  echo "Use 'update' to upgrade, 'diagnose' to check health."
  exit 0
fi
```

3. **Download latest release:**

Get the latest version tag from GitHub API, then download:

```bash
INSTALL_DIR="${ARSHY_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$INSTALL_DIR"

VERSION="0.0.1"  # updated on each release
URL="https://github.com/iZoy/Arshy/releases/download/v${VERSION}/arshy-v${VERSION}-${TARGET}.tar.gz"
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
  echo "  https://github.com/iZoy/Arshy/releases"
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
4. Try a health check: `$HOME/.local/bin/arshy status`
5. Report findings with suggested fixes

### uninstall

```bash
rm -f "$HOME/.local/bin/arshyd" "$HOME/.local/bin/arshy"
# Also kill running daemon
pkill arshyd 2>/dev/null || true
echo "arshy uninstalled"
```
