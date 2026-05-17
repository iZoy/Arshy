#!/bin/bash
# Arshy Bash intercept hook — warns when Bash is called and redirects to arshy_exec.
# This hook is read by Claude Code's PreToolUse mechanism.
#
# Protocol: reads JSON from stdin, writes JSON to stdout.
# Expected input:  {"tool_name":"Bash","tool_input":{"command":"...","description":"..."}}
# Expected output: {"decision":"warn","reason":"Use arshy_exec(action:\"run\",command:\"...\") instead of Bash"}

INPUT=$(cat)
TOOL=$(echo "$INPUT" | python3 -c "import sys,json; print(json.load(sys.stdin).get('tool_name',''))" 2>/dev/null)

if [ "$TOOL" = "Bash" ]; then
  CMD=$(echo "$INPUT" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('tool_input',{}).get('command','unknown command'))" 2>/dev/null)
  echo "{\"decision\":\"warn\",\"reason\":\"Use arshy_exec instead of Bash. arshy_exec(action:\\\"run\\\", command:\\\"$CMD\\\") provides structured output, automatic short/long detection, and task lifecycle management.\"}"
else
  echo '{"decision":"allow"}'
fi
