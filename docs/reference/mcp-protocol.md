# MCP 协议

## 概述

Arshy 实现 MCP (Model Context Protocol) 2024-11-05 规范，通过 stdio 传输。Agent 只需 1 次 MCP 调用即可获得完整结构化结果。

## 初始化

客户端发送 `initialize`，返回 server 能力声明：

```json
{
  "protocolVersion": "2024-11-05",
  "serverInfo": { "name": "arshy", "version": "0.1.0" },
  "capabilities": {
    "tools": { "listChanged": true },
    "resources": { "listChanged": true },
    "prompts": { "listChanged": true },
    "logging": {}
  },
  "instructions": "arshy_exec is your shell for ALL command execution..."
}
```

## 工具

### arshy_exec

统一 shell 执行工具。**Smart Sync：1 次调用 = 完整结果。**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `action` | enum | ✅ | `run` / `cd` / `kill` / `list` / `tail` |
| `command` | string | run/cd 时 | Shell 命令或目录路径（绝对路径） |
| `cwd` | string | | 工作目录（绝对路径，一次性覆盖） |
| `timeout_ms` | integer | | 超时（ms） |
| `mode` | string | | `auto`（默认）/ `sync` / `async` |
| `parse_hint` | string | | 输出格式提示或 parser 名称 |
| `env` | object | | 环境变量 `{"KEY": "value"}` |
| `task_id` | string | kill/tail 时 | 任务 ID |
| `lines` | integer | | tail 行数（默认 50） |
| `format` | string | | tail 格式：`event` / `raw` |
| `status` | string | list 时 | 状态过滤 |
| `limit` | integer | | list 最大条数（默认 10） |

**响应格式（Smart Sync）：**

```json
{
  "content": [{
    "type": "text",
    "text": "{\n  \"status\": \"completed\",\n  \"exit_code\": 0,\n  \"summary\": {...},\n  \"root_cause\": null,\n  \"events\": [...]\n}"
  }]
}
```

### arshy_query

查询结构化事件（高级用法）。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `task_id` | string | ✅ | 任务 ID |
| `event_type` | string | | 事件类型过滤 |
| `severity` | string | | 严重度过滤 |
| `code` | string | | 错误码过滤 |
| `file` | string | | 文件路径过滤 |
| `limit` | integer | | 最大条数（默认 20） |

## 资源

### resources/list

返回最近 50 个任务作为 MCP 资源。

### resources/read

读取指定任务的事件数据。

## 通知

### notifications/message

实时推送任务状态和诊断事件。

| event | 说明 | level |
|-------|------|-------|
| `task_update` | 任务状态变更 | info |
| `task_complete` | 任务完成 | info/error |
| `diagnostic` | 结构化诊断 | error/warning/info |
| `shutdown` | Daemon 即将关闭 | warning |

## 错误响应

所有 MCP error 统一携带 `data.retryable`：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32001,
    "message": "task not found: abc-123",
    "data": { "retryable": false }
  }
}
```

详见 [错误码参考](error-codes.md)。
