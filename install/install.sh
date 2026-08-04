#!/bin/bash
set -e

# Arshy — One-Click Auto Installer
#   install only:          curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh
#   install + enable one agent (ONE LINE):
#                          curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh -s -- --agent codex
#
# Args:
#   --agent <id>    after install, wire arshy into that agent (codex, claude-code,
#                   cursor, vscode, gemini, antigravity, opencode, aider, workbuddy)
#                   and verify with `arshy doctor --agent <id>`
#   --version <tag> release tag to download (default: v0.2.0); ignored when building
#                   from source with cargo
#   --dry-run       print the plan without changing anything

AGENT=""
VERSION="v0.2.0"
DRY_RUN=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --agent) AGENT="${2:-}"; shift 2 ;;
        --version) VERSION="${2:-}"; shift 2 ;;
        --dry-run) DRY_RUN=true; shift ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: install.sh [--agent <id>] [--version <tag>] [--dry-run]"
            exit 1
            ;;
    esac
done

say() {
    if [ "$DRY_RUN" = true ]; then echo "  [dry-run] $*"; else echo "$*"; fi
}
run() { # run "$description" command...
    local desc="$1"; shift
    say "$desc"
    if [ "$DRY_RUN" = true ]; then return 0; fi
    "$@"
}

echo "========================================="
echo "   Arshy Command Center - Auto Installer"
echo "========================================="
[ -n "$AGENT" ] && echo "Target agent: $AGENT"
[ "$DRY_RUN" = true ] && echo "DRY-RUN: nothing will be installed or configured."

# 1. Detect OS & Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"
echo "Detected Environment: $OS ($ARCH)"

INSTALL_DIR="$HOME/.local/bin"
run "create install dir $INSTALL_DIR" mkdir -p "$INSTALL_DIR"

# 2. Build or Fetch Binary
BUILT=false
if command -v cargo >/dev/null 2>&1; then
    echo "Found Rust toolchain. Compiling from source..."
    if [ "$DRY_RUN" = false ]; then
        cargo build --release
        cp target/release/arshy "$INSTALL_DIR/arshy"
        cp target/release/arshyd "$INSTALL_DIR/arshyd"
    else
        say "cargo build --release && cp target/release/{arshy,arshyd} -> $INSTALL_DIR"
    fi
    BUILT=true
else
    echo "Rust compiler not found. Fetching prebuilt release $VERSION..."
    # Artifact naming must match .github/workflows/release.yml:
    # arshy-${VERSION}-${TARGET}.tar.gz with a rust target triple
    # (e.g. arshy-v0.2.0-aarch64-apple-darwin.tar.gz).
    case "$OS-$ARCH" in
        Darwin-arm64|Darwin-aarch64) TARGET="aarch64-apple-darwin" ;;
        Darwin-x86_64) TARGET="x86_64-apple-darwin" ;;
        Linux-x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
        Linux-arm64|Linux-aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
        *) echo "Unsupported platform: $OS ($ARCH)"; exit 1 ;;
    esac
    URL="https://github.com/iZoy/Arshy/releases/download/${VERSION}/arshy-${VERSION}-${TARGET}.tar.gz"
    echo "Downloading binary from: $URL"
    if [ "$DRY_RUN" = false ]; then
        if command -v curl >/dev/null 2>&1; then
            curl -sSL "$URL" | tar -xz -C "$INSTALL_DIR"
        elif command -v wget >/dev/null 2>&1; then
            wget -qO- "$URL" | tar -xz -C "$INSTALL_DIR"
        else
            echo "Error: Neither curl nor wget found in PATH. Cannot download binary."
            exit 1
        fi
        BUILT=true
    else
        say "curl -sSL $URL | tar -xz -C $INSTALL_DIR"
        BUILT=true
    fi
fi

if [ "$BUILT" = false ]; then
    echo "Error: Failed to install Arshy binaries."
    exit 1
fi

say "installed binaries -> $INSTALL_DIR (export PATH=\"$INSTALL_DIR:\$PATH\")"
export PATH="$INSTALL_DIR:$PATH"

# 3. Transparent shell hook shims
run "configure transparent shell hook shims" "$INSTALL_DIR/arshy" hook install

# 4. Daemon auto-start strategy
# arshy uses ON-DEMAND auto-start by design: the daemon is spawned automatically the
# first time an agent runs a command inside a workspace that has opted in (a
# `.arshy.toml` marker or `.arshy/` directory present). There is NO OS-level
# (launchd/systemd) autostart — this is intentional so nothing starts on every boot.
echo "Daemon auto-start: on-demand (no OS-level registration)."
echo "  Opt a workspace in with:  arshy init"
echo "  (optional) OS-level:      arshy install-launchd  /  arshy install-systemd"

# 5. Code-sign binaries (macOS only)
# Ad-hoc code-signing prevents macOS from silently SIGKILL-ing the socket-bound daemon
# binary. NOTE: re-running `cargo build --release && cp` without this step reintroduces
# the risk, so it is part of the installer and should be repeated on manual rebuilds.
if [ "$OS" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
    say "ad-hoc code-signing binaries (macOS)"
    if [ "$DRY_RUN" = false ]; then
        codesign --force --deep -s - "$INSTALL_DIR/arshy" 2>/dev/null || echo "  (codesign arshy skipped)"
        codesign --force --deep -s - "$INSTALL_DIR/arshyd" 2>/dev/null || echo "  (codesign arshyd skipped)"
    fi
fi

# 6. Optional: one-line agent enablement
if [ -n "$AGENT" ]; then
    echo ""
    echo "Enabling arshy for $AGENT..."
    run "arshy setup $AGENT" "$INSTALL_DIR/arshy" setup "$AGENT"
    run "arshy doctor --agent $AGENT (verification)" "$INSTALL_DIR/arshy" doctor --agent "$AGENT"
    echo ""
    echo "Restart $AGENT to activate arshy (arshy_exec / arshy_query become available)."
fi

echo "========================================="
echo "   Installation Completed Successfully! 🎉"
echo "========================================="
echo "Please restart your terminal or run:"
echo "   source ~/.zshrc (or ~/.bashrc)"
echo ""
echo "To check installation status, run:"
echo "   arshy status"
echo ""
echo "Drop your custom TOML parser definitions into:"
echo "   ~/.arshy/parsers/"
echo "========================================="
