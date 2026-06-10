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

# Extract tool name and command
TOOL=$(echo "$INPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('tool_name',''))" 2>/dev/null || echo "")
COMMAND=$(echo "$INPUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d.get('tool_input',{}).get('command',''))" 2>/dev/null || echo "")

# Only intercept Bash tool calls
if [ "$TOOL" != "Bash" ]; then
    echo '{"decision":"approve"}'
    exit 0
fi

# Don't intercept arshy commands (already going through arshy)
if echo "$COMMAND" | grep -qE '^\s*(arshy|arshyd)\b'; then
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

# Route through arshy
# Use --format json for structured output, then extract the relevant info
ARSHY_OUTPUT=$(arshy run "$COMMAND" --format json 2>&1)
ARSHY_EXIT=$?

# Extract status and output
STATUS=$(echo "$ARSHY_OUTPUT" | python3 -c "
import json,sys
d=json.load(sys.stdin)
print(d.get('status','unknown'))
" 2>/dev/null || echo "unknown")

if [ "$STATUS" = "completed" ] || [ "$STATUS" = "failed" ]; then
    # Extract the key information for the agent
    SUMMARY=$(echo "$ARSHY_OUTPUT" | python3 -c "
import json,sys
d=json.load(sys.stdin)
result = {
    'status': d.get('status'),
    'exit_code': d.get('exit_code'),
    'duration_ms': d.get('duration_ms'),
}
# For short commands, include raw output
if d.get('short_command') and d.get('raw_output'):
    result['output'] = d['raw_output']
# For structured commands, include summary + errors
elif d.get('summary'):
    result['summary'] = d['summary']
    errors = [e for e in d.get('events',[]) if e.get('severity')=='error']
    if errors:
        result['errors'] = [{'message': e.get('message',''), 'code': e.get('code')} for e in errors[:5]]
    if d.get('root_cause'):
        result['root_cause'] = d['root_cause'].get('message','')
print(json.dumps(result, indent=2))
" 2>/dev/null || echo "$ARSHY_OUTPUT")

    # Return structured result with approval
    cat <<HOOKEOF
{
  "decision": "approve",
  "reason": "Routed through arshy. Status: $STATUS",
  "output": $(echo "$SUMMARY" | python3 -c "import json,sys; print(json.dumps(sys.stdin.read()))" 2>/dev/null || echo '""')
}
HOOKEOF
else
    # arshy failed, fall through to native bash
    echo '{"decision":"approve"}'
fi
