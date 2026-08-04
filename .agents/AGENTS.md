# Arshy Execution Rules

- When executing terminal or shell commands, you MUST call the `arshy_exec` tool (from the `arshy` MCP server) instead of the native `run_command` tool.
- Always use `arshy_exec` to run builds, tests, linters, or general commands to benefit from structured parsing, token optimization, and automatic diagnostics.
- To view logs or query past command execution details, use the `arshy_query` tool.
