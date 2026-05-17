#!/bin/bash
# Arshy Bash intercept hook — OPTIONAL enforcement tool.
# Not enabled by default. For users who want strict "no Bash" enforcement,
# add this to ~/.claude/settings.json:
#
#   "hooks": {
#     "PreToolUse": [
#       {
#         "tool": "Bash",
#         "hook": "~/.claude/plugins/arshy/hooks/bash-intercept.sh"
#       }
#     ]
#   }
#
# Without this hook, arshy relies on MCP instructions + tool descriptions
# to guide the agent — silent, no popups, no friction.

INPUT=$(cat)
TOOL=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tool_name',''))" 2>/dev/null)

if [ "$TOOL" = "Bash" ]; then
  CMD=$(echo "$INPUT" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_input',{}).get('command','unknown command'))" 2>/dev/null)
  echo "{\"decision\":\"warn\",\"reason\":\"Use arshy_exec instead of Bash. arshy_exec(action:\\\"run\\\", command:\\\"$CMD\\\") provides structured output, automatic short/long detection, and task lifecycle management.\"}"
else
  echo '{"decision":"allow"}'
fi
