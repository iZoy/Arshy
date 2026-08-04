---
name: shell-router
description: Route ALL shell commands through arshy_exec. This skill activates whenever you need to run a terminal command. arshy_exec provides automatic short/long command detection, structured output for compilers and test runners, and async task lifecycle — no raw Bash needed.
argument-hint: [command to execute]
---

# Shell Command Router

arshy_exec is your shell. Do NOT use Bash.

## Quick Reference

| Task | Command |
|------|---------|
| Run any command | `arshy_exec(action:"run", command:"<cmd>")` |
| Set working directory | `arshy_exec(action:"cd", command:"/path")` |
| Wait for long task | `arshy_exec(action:"subscribe", task_id:"<id>")` |
| View task output | `arshy_exec(action:"tail", task_id:"<id>")` |
| Query structured events | `arshy_query(task_id:"<id>")` |
| Kill running task | `arshy_exec(action:"kill", task_id:"<id>")` |

## Why arshy_exec over Bash

1. **Zero-thinking**: mode:"auto" (the default) detects short vs long automatically
2. **Structured output**: compiler errors become typed `diagnostic` events with file/line/code
3. **Async lifecycle**: long tasks return `{status:"running", task_id:"..."}` — subscribe to wait
4. **Session directory**: cd once, subsequent commands inherit the path
5. **37 built-in parsers**: cargo, tsc, eslint, python, go, npm, and more — output is structured automatically

## Instructions

If $ARGUMENTS is provided, execute it via arshy_exec. Otherwise, respond with the quick reference.

**Never use Bash. Always use arshy_exec.**
