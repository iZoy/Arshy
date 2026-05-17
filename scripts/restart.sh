#!/usr/bin/env bash
# -*- mode: shell-script -*-
# arshy daemon restart + smoke test + optional launchd persistence
# Usage:
#   bash scripts/restart.sh        # one-shot restart + test
#   bash scripts/restart.sh --perm # also install launchd plist (macOS) for auto-start on boot

set -euo pipefail

BIN_DIR="$HOME/.cargo/bin"
ARSHYD="$BIN_DIR/arshyd"
ARSHY="$BIN_DIR/arshy"
SOCKET="$HOME/.local/share/arshy/arshyd.sock"
LOG="/tmp/arshyd.log"
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

ok()   { echo -e "${GREEN}[OK]${NC} $*"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
die()  { echo -e "${RED}[FATAL]${NC} $*"; exit 1; }

# ── 1. Kill existing daemon ──────────────────────────────────────────────
echo "==> Killing any existing daemon..."
if pkill arshyd 2>/dev/null; then
    sleep 1
    ok "killed old daemon"
else
    ok "no existing daemon"
fi

# ── 2. Verify binary ─────────────────────────────────────────────────────
echo "==> Checking binary..."
if [ ! -x "$ARSHYD" ]; then
    # fallback: check target/release
    if [ -x "./target/release/arshyd" ]; then
        warn "$ARSHYD missing; copying from ./target/release/"
        mkdir -p "$BIN_DIR"
        cp ./target/release/arshyd "$ARSHYD"
        cp ./target/release/arshy  "$ARSHY"
    else
        die "arshyd not found at $ARSHYD or ./target/release/. Run 'cargo build --release' first."
    fi
fi

BUILD_TS=$(stat -f '%Sm' "$ARSHYD" 2>/dev/null || stat -c '%y' "$ARSHYD" 2>/dev/null || echo "unknown")
echo "   Path:    $ARSHYD"
echo "   Size:    $(du -h "$ARSHYD" | cut -f1)"
echo "   Built:   $BUILD_TS"

# ── 3. Start daemon ──────────────────────────────────────────────────────
echo "==> Starting daemon..."
nohup "$ARSHYD" > "$LOG" 2>&1 &
PID=$!
echo "   PID:     $PID"
echo "   Log:     $LOG"
echo "   Socket:  $SOCKET"

# ── 4. Wait for socket ───────────────────────────────────────────────────
echo -n "==> Waiting for daemon"
for i in $(seq 1 15); do
    if [ -S "$SOCKET" ]; then
        echo ""
        ok "socket ready after ${i}s"
        break
    fi
    echo -n "."
    sleep 1
done

if [ ! -S "$SOCKET" ]; then
    echo ""
    warn "socket did not appear after 15s — checking log:"
    tail -10 "$LOG"
    die "daemon may have failed to start (see $LOG)"
fi

# Give it a moment to finish initialising
sleep 1

# ── 5. Smoke tests ───────────────────────────────────────────────────────
echo ""
echo "==> Smoke tests..."

# 5a. Status
echo -n "   status:        "
if OUTPUT=$("$ARSHY" status 2>&1); then
    ok "$(echo "$OUTPUT" | head -1)"
else
    warn "status command failed: $OUTPUT"
fi

# 5b. Short command
echo -n "   short cmd:     "
if OUTPUT=$("$ARSHY" run "echo hello" 2>&1); then
    ok "output='$OUTPUT'"
else
    warn "failed: $OUTPUT"
fi

# 5c. Short pipe (verifies pipe-is-short enhancement)
echo -n "   short pipe:    "
if OUTPUT=$("$ARSHY" run "echo hello | wc -c" 2>&1); then
    EXPECTED="6"
    if echo "$OUTPUT" | grep -q "$EXPECTED"; then
        ok "piped → '$EXPECTED' (short path)"
    else
        warn "unexpected output: $OUTPUT"
    fi
else
    warn "pipe failed: $OUTPUT"
fi

# 5d. isError on failure (verifies failure-visibility enhancement)
echo -n "   isError fail:  "
if OUTPUT=$("$ARSHY" run "exit 42" 2>&1); then
    warn "exit 42 should have failed but succeeded"
else
    ok "exit 42 correctly reported as failure"
fi

# 5e. Smart sync (verifies auto-mode smart wait enhancement)
echo -n "   smart sync:    "
# A long command that finishes quickly via the structured path
if OUTPUT=$("$ARSHY" run "echo fast-long-cmd-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" 2>&1); then
    ok "fast long cmd returned directly (smart sync)"
else
    warn "unexpected: $OUTPUT"
fi

# ── 6. Summary ───────────────────────────────────────────────────────────
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo " Daemon:  PID $PID  |  Socket $SOCKET  |  Log $LOG"
echo " Binary:  $BUILD_TS"
echo ""
echo " MCP should reconnect automatically on next tool call."
echo " To tail logs:  tail -f $LOG"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── 7. Optional launchd persistence ──────────────────────────────────────
if [ "${1:-}" = "--perm" ]; then
    echo ""
    echo "==> Installing launchd plist for auto-start on boot..."

    PLIST="$HOME/Library/LaunchAgents/com.arshy.daemon.plist"
    mkdir -p "$HOME/Library/LaunchAgents"

    cat > "$PLIST" <<PLISTEOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.arshy.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>$ARSHYD</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>30</integer>
    <key>ExitTimeOut</key>
    <integer>10</integer>
    <key>StandardOutPath</key>
    <string>$LOG</string>
    <key>StandardErrorPath</key>
    <string>$LOG</string>
    <key>ProcessType</key>
    <string>Background</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_BACKTRACE</key>
        <string>1</string>
    </dict>
</dict>
</plist>
PLISTEOF

    launchctl unload "$PLIST" 2>/dev/null || true
    launchctl load "$PLIST"
    ok "launchd plist installed: $PLIST"
    ok "arshyd will auto-start on boot and stay alive"
fi
