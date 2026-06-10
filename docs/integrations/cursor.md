# Cursor Integration

arshy works with Cursor via MCP. Add to `.cursor/mcp.json` in your project:

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

Or add to global Cursor settings (`~/.cursor/mcp.json`).

## Usage

Once configured, Cursor's AI agent will automatically use `arshy_exec` and
`arshy_query` tools for shell command execution with structured output.

- Build errors are returned with file, line, column, and severity
- `--errors-only` flag filters to error-level events only
- Failed commands include `raw_output_ref` for full output retrieval
