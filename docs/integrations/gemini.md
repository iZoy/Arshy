# Google Gemini CLI Integration

arshy works with Gemini CLI via MCP. Add to your Gemini settings:

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

Gemini CLI will use `arshy_exec` and `arshy_query` for shell execution
with structured error/warning output.
