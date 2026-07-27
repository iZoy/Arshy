#!/bin/bash
set -e

# Arshy — One-Click Auto Installer
# Installs arshy binary, sets up the background daemon, and activates transparent shell hook.

echo "========================================="
echo "   Arshy Command Center - Auto Installer"
echo "========================================="

# 1. Detect OS & Architecture
OS="$(uname -s)"
ARCH="$(uname -m)"
echo "Detected Environment: $OS ($ARCH)"

INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR"

# 2. Build or Fetch Binary
BUILT=false
if command -v cargo >/dev/null 2>&1; then
    echo "Found Rust toolchain. Compiling from source..."
    cargo build --release
    cp target/release/arshy "$INSTALL_DIR/arshy"
    cp target/release/arshyd "$INSTALL_DIR/arshyd"
    BUILT=true
else
    # Fallback to downloading release binary
    VERSION="v0.2.0"
    echo "Rust compiler not found. Fetching prebuilt release $VERSION..."
    
    # Map OS / ARCH to release naming convention
    case "$OS" in
        Darwin) SYS_NAME="macos" ;;
        Linux) SYS_NAME="linux" ;;
        *) echo "Unsupported operating system: $OS"; exit 1 ;;
    esac

    case "$ARCH" in
        x86_64) ARCH_NAME="amd64" ;;
        arm64|aarch64) ARCH_NAME="arm64" ;;
        *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac

    URL="https://github.com/iZoy/Arshy/releases/download/${VERSION}/arshy-${SYS_NAME}-${ARCH_NAME}.tar.gz"
    echo "Downloading binary from: $URL"
    
    if command -v curl >/dev/null 2>&1; then
        curl -sSL "$URL" | tar -xz -C "$INSTALL_DIR"
        BUILT=true
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "$URL" | tar -xz -C "$INSTALL_DIR"
        BUILT=true
    else
        echo "Error: Neither curl nor wget found in PATH. Cannot download binary."
        exit 1
    fi
fi

if [ "$BUILT" = false ]; then
    echo "Error: Failed to install Arshy binaries."
    exit 1
fi

echo "Installed binaries successfully to $INSTALL_DIR"
export PATH="$INSTALL_DIR:$PATH"

# 3. Setup Shell Interception Wrapper
echo "Configuring transparent shell hook shims..."
"$INSTALL_DIR/arshy" hook install

# 4. Setup Daemon Autostart
echo "Configuring background daemon autostart..."
if [ "$OS" = "Darwin" ]; then
    # Install macOS Launchd plist
    "$INSTALL_DIR/arshy" install-launchd
    echo "Daemon service registered under macOS launchd."
    # Launch launchd agent
    launchctl bootstrap gui/$(id -u) "$HOME/Library/LaunchAgents/com.arshy.daemon.plist" || true
    launchctl kickstart -k gui/$(id -u)/com.arshy.daemon || true
elif [ "$OS" = "Linux" ]; then
    # Install Linux systemd service
    "$INSTALL_DIR/arshy" install-systemd
    echo "Daemon service registered under Linux systemd."
    systemctl --user daemon-reload || true
    systemctl --user enable arshyd.service || true
    systemctl --user restart arshyd.service || true
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
