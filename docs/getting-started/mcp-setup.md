# 接入 MCP

## Claude Code

```bash
arshy install
```

自动写入 Claude Code MCP 配置。重启 Claude Code 生效。

手动配置（`~/.claude/claude_config.json`）：

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

## Cursor

在 `.cursor/mcp.json` 中添加：

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

## 验证

在 Claude Code 中执行：

```
> 用 arshy 执行 ls -la
```

Agent 会通过 `arshy_exec` 工具执行命令并返回结构化输出。

## 工具模型

Agent 看到 2 个工具：

| 工具 | 用途 |
|------|------|
| `arshy_exec` | 执行命令、管理任务（run/kill/list/tail） |
| `arshy_query` | 查询任务的结构化事件 |

Agent 不需要选择工具 — `arshy_exec` 配合 `action` 参数覆盖所有操作。

## 卸载

```bash
arshy uninstall
```
