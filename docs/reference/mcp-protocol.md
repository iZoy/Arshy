# MCP 协议

## 概述

Arshy 实现 MCP (Model Context Protocol) 2024-11-05 规范，通过 stdio 传输。初始化时宣告 `experimental.preferredShell` 能力和权威 `instructions`，Agent 自然获知 arshy 是主 shell。

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
    "logging": {},
    "experimental": {
      "preferredShell": true,
      "preferredOver": ["bash", "shell", "run_terminal_cmd"],
      "description": "arshy is a structured shell replacement. It automatically detects short vs long commands, provides structured output for build/test tools, and supports async task lifecycle. Agents should route ALL shell commands through arshy_exec instead of raw Bash."
    }
  },
  "instructions": "arshy_exec is your shell for ALL command execution. Route every shell command through arshy_exec(action:\"run\", command:\"<cmd>\") — do not use raw Bash unless arshy is unreachable..."
}
```

**关键字段：**
- `experimental.preferredShell` — 结构化能力宣告，供客户端自动发现和提升优先级
- `instructions` — 权威声明："I am your shell"，agent 自然优先使用

## 工具

### arshy_exec

统一 shell 执行工具。**action 参数分发全部操作。**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `action` | enum | ✅ | `run` / `cd` / `kill` / `list` / `tail` / `subscribe` |
| `command` | string | run/cd 时 | Shell 命令或目录路径（绝对路径） |
| `cwd` | string | | 工作目录（绝对路径，一次性覆盖；用 cd 设置会话级） |
| `timeout_ms` | integer | | 超时（ms） |
| `mode` | string | | `auto`（默认）/ `sync` / `async` |
| `parse_hint` | string | | 输出格式提示（json/csv/table/raw）或 parser 名称（python/cargo） |
| `env` | object | | 环境变量 `{"KEY": "value"}` |
| `task_id` | string | kill/tail/subscribe 时 | 任务 ID |
| `lines` | integer | | tail 行数（默认 50） |
| `format` | string | | tail 格式：`event` / `raw` |
| `status` | string | list 时 | 状态过滤：running/completed/failed/killed |
| `limit` | integer | | list 最大条数（默认 10） |

**短命令响应（mode=auto 且命中短路径）：**

纯文本返回，与原生 Bash 一致：

```json
{ "content": [{ "type": "text", "text": "total 48\ndrwxr-xr-x ..." }] }
```

失败时带 `isError: true` + `[command failed: exit code N]`。

**长命令响应（<2s 完成）：**

```json
{
  "content": [{
    "type": "text",
    "text": "{\"task_id\":\"b80c...\",\"status\":\"completed\",\"exit_code\":0,\"duration_ms\":7350,\"event_count\":334}"
  }]
}
```

**长命令响应（>2s 运行中）：**

```json
{
  "content": [{
    "type": "text",
    "text": "{\"task_id\":\"b80c...\",\"status\":\"running\"}"
  }]
}
```

→ Agent 调用 `arshy_exec(action:"subscribe", task_id:"...")` 等待完成。

### arshy_query

查询任务的结构化事件。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `task_id` | string | ✅ | 任务 ID |
| `event_type` | string | | 事件类型过滤（diagnostic/location/test_result/crash/summary） |
| `severity` | string | | 严重度过滤（error/warning/info） |
| `code` | string | | 错误码过滤（如 E0308、TS2345） |
| `file` | string | | 文件路径过滤 |
| `limit` | integer | | 最大条数（默认 20） |

响应示例：

```json
{
  "events": [
    {
      "type": "diagnostic",
      "severity": "error",
      "code": "E0308",
      "message": "mismatched types",
      "location": { "file": "src/main.rs", "line": 10, "column": 5 }
    }
  ],
  "total": 42
}
```

## 资源

### resources/list

返回最近 50 个任务作为 MCP 资源：

```json
{
  "resources": [{
    "uri": "arshy://task/{task_id}",
    "name": "cargo build [completed]",
    "description": "Task {id} — cargo build",
    "mimeType": "application/json"
  }]
}
```

### resources/read

读取指定任务的事件数据。

## 通知

### notifications/message

实时推送任务状态和诊断事件。

| event | 说明 | level |
|-------|------|-------|
| `task_update` | 任务状态变更 | info |
| `task_complete` | 任务完成（含 exit_code） | info/error |
| `diagnostic` | Parser 产生的结构化诊断 | error/warning/info |
| `batch` | 批量通知（100ms 窗口汇聚） | info |
| `shutdown` | Daemon 即将关闭 | warning |

### notifications/cancelled

Agent 取消操作时，proxy 自动查找 requestId 对应的 task_id 并 kill。

## Prompts

3 个内置 prompt 模板：

| Prompt | 用途 |
|--------|------|
| `analyze_build_failure` | 分析构建失败，输入 command + output |
| `diagnose_test_failure` | 诊断测试失败，输入 task_id |
| `review_task_output` | 汇总任务的结构化事件输出 |

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
