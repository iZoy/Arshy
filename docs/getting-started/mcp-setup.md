# 接入 MCP

## 自动接入（Plugin 安装）

```bash
claude plugin install arshy@arshy-marketplace
```

安装后重启 Claude Code。Arshy 的 MCP server 在初始化时自动：

1. **宣告 shell 主权**：`instructions` 声明 "arshy_exec is your shell for ALL command execution"
2. **注册 2 个工具**：`arshy_exec`（统一 shell）+ `arshy_query`（事件查询）
3. **声明实验能力**：`experimental.preferredShell: true` + `preferredOver: ["bash", "shell"]`
4. **提供 3 个 prompt 模板**：构建失败分析、测试失败诊断、任务输出审查
5. **暴露任务资源**：`arshy://task/{id}` URI 可通过 MCP resources 读取

**无需 CLAUDE.md 配置**。Agent 从 MCP 指令中自然获知 arshy 是 shell。

## 手动配置

如果不用 plugin，手动在 `~/.claude/settings.json` 中添加：

```json
{
  "mcpServers": {
    "arshy": {
      "command": "~/.cargo/bin/arshy",
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

安装后重启 Claude Code。Agent 的工具列表中会出现 `arshy_exec` 和 `arshy_query`。

## 初始化响应（当前版本）

```json
{
  "protocolVersion": "2024-11-05",
  "serverInfo": { "name": "arshy", "version": "0.1.0" },
  "capabilities": {
    "tools": { "listChanged": true },
    "resources": { "listChanged": true },
    "prompts": { "listChanged": true },
    "logging": {},
    "experimental": {
      "preferredShell": true,
      "preferredOver": ["bash", "shell", "run_terminal_cmd"],
      "description": "arshy is a structured shell replacement..."
    }
  },
  "instructions": "arshy_exec is your shell for ALL command execution. Route every shell command through arshy_exec(action:\"run\", command:\"<cmd>\") — do not use raw Bash unless arshy is unreachable..."
}
```

## 卸载

```bash
claude plugin uninstall arshy
# 或
arshy uninstall
```
