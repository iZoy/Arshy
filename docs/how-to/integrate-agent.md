# Integrate an MCP client

Arshy has one integration surface: the standard MCP stdio server.

```sh
arshy mcp config
```

Copy the JSON output into the MCP client's configuration. The entry is always:

```json
{ "command": "arshy", "args": ["mcp", "serve"] }
```

The client owns its configuration and lifecycle. Arshy does not inspect agent
names, write hooks, edit `AGENTS.md`, install shell shims, or provide agent
specific uninstall behavior. To remove the integration, delete this one MCP
entry in the client.
