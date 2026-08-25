# FAQ

### Does Arshy support my agent?

If the client supports MCP stdio, add `arshy mcp serve` to its MCP servers.
There is no agent allowlist or custom adapter.

### Does it save tokens?

Arshy does not publish a token-saving estimate. Use `stats`, `analyze`, or
`benchmark` to inspect quality-v1 engineering components.

### Is it a sandbox?

No. `security.allowed_cwds` is only a working-directory guard. Use an
OS/container sandbox when stronger isolation is required.

### How do I uninstall it?

Remove the `arshy` MCP entry from your client and delete the two binaries from
the install directory. Arshy does not leave hooks or generated project files.
