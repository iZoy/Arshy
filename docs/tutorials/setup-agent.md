# MCP client setup

This page replaces the former agent-specific integration guide. Arshy does
not modify a named agent, shell hooks, or project instructions.

Install the binaries, then add this generic server entry to any MCP client:

```json
{
  "mcpServers": {
    "arshy": { "command": "arshy", "args": ["mcp", "serve"] }
  }
}
```

Use `arshy mcp config` to print the same snippet. Verify the installation with
`arshy doctor`, then call the server's execution/query tools from the client.
