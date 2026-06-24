#!/bin/bash
# shell/install.sh — Install arshy shell wrapper
#
# Usage: bash shell/install.sh
#
# What it does:
#   1. Copies wrapper.sh to ~/.arshy/shell.sh
#   2. Adds `source ~/.arshy/shell.sh` to your shell rc file
#   3. Idempotent — safe to run multiple times

set -euo pipefail

WRAPPER_SRC="$(cd "$(dirname "$0")" && pwd)/wrapper.sh"
INSTALL_DIR="$HOME/.arshy"
INSTALL_PATH="$INSTALL_DIR/shell.sh"
SOURCE_LINE='[ -f ~/.arshy/shell.sh ] && source ~/.arshy/shell.sh'

echo "Installing arshy shell wrapper..."

# 1. Create install dir and copy wrapper
mkdir -p "$INSTALL_DIR"
cp "$WRAPPER_SRC" "$INSTALL_PATH"
chmod +x "$INSTALL_PATH"
echo "  ✓ Copied wrapper to $INSTALL_PATH"

# 2. Detect shell and rc file
SHELL_NAME="$(basename "$SHELL")"
case "$SHELL_NAME" in
    zsh)  RC_FILE="$HOME/.zshrc" ;;
    bash) RC_FILE="$HOME/.bashrc" ;;
    *)
        echo "  ⚠ Unsupported shell: $SHELL_NAME"
        echo "    Manually add this line to your shell rc file:"
        echo "    $SOURCE_LINE"
        exit 0
        ;;
esac

# 3. Add source line if not already present
if [ -f "$RC_FILE" ] && grep -qF "shell.sh" "$RC_FILE" 2>/dev/null; then
    echo "  ✓ Already installed in $RC_FILE"
else
    echo "" >> "$RC_FILE"
    echo "# arshy shell wrapper — route commands through arshy daemon" >> "$RC_FILE"
    echo "$SOURCE_LINE" >> "$RC_FILE"
    echo "  ✓ Added to $RC_FILE"
fi

echo ""
echo "Done! Restart your shell or run: source $RC_FILE"
echo "Then use: a <command>  (e.g., a cargo build --release)"
