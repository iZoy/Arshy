# GitHub Copilot CLI Integration

arshy works with Copilot CLI via MCP. Add to your Copilot config:

```json
{
  "mcpServers": {
    "arshy": {
      "command": "arshy",
      "args": ["--from-mcp"]
    }
  }
}
```

## Usage

Copilot CLI will use `arshy_exec` for shell commands, providing structured
diagnostic output instead of raw terminal text.
