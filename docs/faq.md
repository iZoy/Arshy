# FAQ

### Does Arshy support my agent?

If the client supports MCP stdio, run `arshy mcp config --format prompt` and
send the complete output to the Agent in the current project. It uses the
client's native MCP configuration, detects conflicts, and reports any manual
step. For a manual fallback, use `arshy mcp config --format json`.

### Does it save tokens?

Arshy does not publish a token-saving estimate. Use `stats`, `analyze`, or
`benchmark` to inspect quality-v2 engineering components.

### Is it a sandbox?

No. `security.allowed_cwds` is only a working-directory guard. Use an
OS/container sandbox when stronger isolation is required.

### How do I uninstall it?

Remove the `arshy` MCP entry from your client, remove only any project-rule
change the Agent explicitly made for this setup, then delete the two binaries
from the install directory. The setup Prompt never creates hooks or shell
shims.
