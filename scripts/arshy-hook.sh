#!/bin/bash
# scripts/arshy-hook.sh — PreToolUse hook for Claude Code
# Intercepts Bash tool calls and routes them through arshy.
#
# Install:
#   cp scripts/arshy-hook.sh ~/.claude/hooks/arshy-hook.sh
#   chmod +x ~/.claude/hooks/arshy-hook.sh
#
# Configure in ~/.claude/settings.json:
# {
#   "hooks": {
#     "PreToolUse": [
#       {
#         "matcher": "Bash",
#         "hook": "~/.claude/hooks/arshy-hook.sh"
#       }
#     ]
#   }
# }

set -euo pipefail

# Read JSON input from stdin
INPUT=$(cat)

# Extract tool name and command using python (safe JSON parsing)
TOOL=$(printf '%s' "$INPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('tool_name',''))" 2>/dev/null || echo "")
COMMAND=$(printf '%s' "$INPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('tool_input',{}).get('command',''))" 2>/dev/null || echo "")

# Only intercept Bash tool calls
if [ "$TOOL" != "Bash" ]; then
    echo '{"decision":"approve"}'
    exit 0
fi

# Don't intercept arshy commands (already going through arshy)
if printf '%s' "$COMMAND" | grep -qE '^\s*(arshy|arshyd)\b'; then
    echo '{"decision":"approve"}'
    exit 0
fi

# Don't intercept empty commands
if [ -z "$COMMAND" ]; then
    echo '{"decision":"approve"}'
    exit 0
fi

# Check if daemon is running
if ! pgrep -f arshyd >/dev/null 2>&1; then
    # Try to start daemon
    arshyd &>/dev/null &
    sleep 1
    if ! pgrep -f arshyd >/dev/null 2>&1; then
        # Daemon failed to start, fall through to native bash
        echo '{"decision":"approve"}'
        exit 0
    fi
fi

# Route through arshy (quote "$COMMAND" to prevent word splitting)
ARSHY_OUTPUT=$(arshy run "$COMMAND" --format json 2>&1) || true

# Build safe JSON output using python (no shell interpolation in JSON)
printf '%s' "$ARSHY_OUTPUT" | python3 -c "
import json, sys

try:
    d = json.load(sys.stdin)
except:
    print('{\"decision\": \"approve\"}')
    sys.exit(0)

status = d.get('status', 'unknown')

if status in ('completed', 'failed'):
    result = {
        'status': d.get('status'),
        'exit_code': d.get('exit_code'),
        'duration_ms': d.get('duration_ms'),
    }
    if d.get('short_command') and d.get('raw_output'):
        result['output'] = d['raw_output']
    elif d.get('summary'):
        result['summary'] = d['summary']
        errors = [e for e in d.get('events', []) if e.get('severity') == 'error']
        if errors:
            result['errors'] = [{'message': e.get('message', ''), 'code': e.get('code')} for e in errors[:5]]
        if d.get('root_cause'):
            result['root_cause'] = d['root_cause'].get('message', '')

    output = json.dumps(result)
    hook_result = {
        'decision': 'approve',
        'reason': f'Routed through arshy. Status: {status}',
        'output': output,
    }
    print(json.dumps(hook_result))
else:
    print('{\"decision\": \"approve\"}')
"
