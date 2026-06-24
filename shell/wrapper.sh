#!/bin/bash
# shell/wrapper.sh — Transparent bash takeover via arshy daemon
#
# Usage: source this file in your .zshrc/.bashrc, then use `a` prefix:
#   a cargo build --release
#   a git status
#   a ls -la
#
# Falls back to native bash if daemon is down.

# Socket path (matches daemon default)
_ARSHY_SOCK="${XDG_DATA_HOME:-$HOME/.local/share}/arshy/arshyd.sock"
# Cache daemon status for 5s to avoid per-command socket checks
_ARSHY_ALIVE_UNTIL=0

_arshy_is_alive() {
    local now
    now=$(date +%s)
    if [ "$now" -lt "$_ARSHY_ALIVE_UNTIL" ]; then
        return 0
    fi
    if [ -S "$_ARSHY_SOCK" ]; then
        _ARSHY_ALIVE_UNTIL=$((now + 5))
        return 0
    fi
    return 1
}

a() {
    local cmd="$*"

    # Empty command → show help
    if [ -z "$cmd" ]; then
        echo "arshy shell wrapper — route commands through arshy daemon"
        echo "Usage: a <command> [args...]"
        echo "Example: a cargo build --release"
        return 0
    fi

    # Interactive commands → run natively (they need a TTY)
    case "$1" in
        vim|nvim|vi|top|htop|ssh|mosh|less|more|man|nano|pico|emacs)
            command "$@"
            return $?
            ;;
        sudo)
            # sudo with -i/-s is interactive; sudo with a command goes through arshy
            case "$2" in
                -i|-s) command "$@"; return $? ;;
            esac
            ;;
    esac

    # Daemon down → fallback to native bash
    if ! _arshy_is_alive; then
        command "$@"
        return $?
    fi

    # Route through arshy daemon
    local output exit_code raw

    # Use --cwd to tell daemon the real working directory
    output=$(arshy run "$cmd" --format json --cwd "$PWD" 2>/dev/null) || {
        # arshy itself failed → fallback
        command "$@"
        return $?
    }

    # Extract exit code
    exit_code=$(printf '%s' "$output" | python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)
    print(d.get('exit_code', 1))
except:
    print(1)
" 2>/dev/null)

    # Extract output: raw for short commands, content text for long commands
    printf '%s' "$output" | python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)
    if d.get('short_command'):
        print(d.get('raw_output', ''), end='')
    else:
        content = d.get('content', [])
        if content and isinstance(content, list):
            print(content[0].get('text', ''), end='')
        # Also print root cause if present
        rc = d.get('root_cause')
        if rc and rc.get('message'):
            print()
            print('Root cause:', rc['message'])
except:
    pass
" 2>/dev/null

    return "${exit_code:-1}"
}
