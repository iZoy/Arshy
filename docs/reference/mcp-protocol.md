# MCP 协议

## 概述

Arshy 实现 MCP (Model Context Protocol) 2024-11-05 规范，通过 stdio 传输。

## 初始化

客户端发送 `initialize`，返回 server 能力声明：

```json
{
  "protocolVersion": "2024-11-05",
  "serverInfo": { "name": "arshy", "version": "0.1.0" },
  "capabilities": {
    "tools": {},
    "resources": { "listChanged": true },
    "notifications": { "diagnostic": {} }
  },
  "instructions": {
    "shell_execution": "NEVER use raw shell tools. ALL commands go through arshy_exec.",
    "mode_guidance": "Use mode:\"auto\". Arshy automatically detects short vs long commands."
  }
}
```

## 工具

### arshy_exec

统一执行/管理工具。

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `action` | enum | ✅ | `run` / `kill` / `list` / `tail` |
| `command` | string | run 时 | Shell 命令 |
| `cwd` | string | | 工作目录 |
| `timeout_ms` | integer | | 超时 |
| `mode` | string | | `auto`（默认）/ `sync` / `async` |
| `parse_hint` | string | | `json` / `csv` / `table` / `raw` |
| `task_id` | string | kill/tail 时 | 任务 ID |
| `lines` | integer | | tail 行数（默认 50） |
| `format` | string | | tail 格式：`event` / `raw` |
| `status` | string | list 时 | 状态过滤 |
| `limit` | integer | | list 最大条数（默认 10） |

**短命令响应（mode=auto 且短命令）：**

```json
{ "content": [{ "type": "text", "text": "hello world\n" }] }
```

**长命令响应：**

```json
{ "content": [{ "type": "text", "text": "{\"task_id\":\"...\",\"status\":\"completed\",...}" }] }
```

### arshy_query

查询任务的结构化事件。

**参数：**

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

返回最近 50 个任务作为 MCP 资源：

```json
{
  "resources": [
    {
      "uri": "arshy://task/{task_id}",
      "name": "cargo build [completed]",
      "description": "Task {id} — cargo build",
      "mimeType": "application/json"
    }
  ]
}
```

### resources/read

读取指定任务的事件：

```json
{
  "contents": [{
    "uri": "arshy://task/{task_id}",
    "mimeType": "application/json",
    "text": "{\"task_id\":\"...\",\"total_events\":42,\"events\":[...]}"
  }]
}
```

## 通知

### notifications/message

实时推送任务状态变更：

```json
{
  "method": "notifications/message",
  "params": {
    "level": "info",
    "logger": "arshy",
    "data": {
      "event": "task_update",
      "task_id": "abc-123",
      "status": "running"
    }
  }
}
```

**事件类型：**

| event | 说明 | level |
|-------|------|-------|
| `task_update` | 任务状态变更 | info |
| `task_complete` | 任务完成 | info/error |
| `diagnostic` | 解析器诊断 | error/warning/info |
| `batch` | 批量通知 | info |
| `shutdown` | daemon 关闭 | warning |

### notifications/cancelled

Agent 取消操作时发送，arshy 调用 `arshy_kill` 终止对应任务。

```json
{
  "method": "notifications/cancelled",
  "params": {
    "requestId": 42,
    "reason": "user cancelled"
  }
}
```

收到后，proxy 自动查找该 requestId 关联的 task_id 并发送 kill 请求。

## Prompts

### prompts/list

返回可用的 prompt 模板：

```json
{
  "prompts": [
    {
      "name": "analyze_build_failure",
      "description": "Help me understand why this build failed and suggest fixes.",
      "arguments": [
        { "name": "command", "description": "The build command that failed", "required": true },
        { "name": "output", "description": "The build output or error log", "required": false }
      ]
    },
    {
      "name": "diagnose_test_failure",
      "description": "Help me understand why these tests failed and suggest next steps.",
      "arguments": [
        { "name": "task_id", "description": "The task ID of the failed test run", "required": true }
      ]
    },
    {
      "name": "review_task_output",
      "description": "Review the structured output of a command and summarize findings.",
      "arguments": [
        { "name": "task_id", "description": "The task ID to review", "required": true }
      ]
    }
  ]
}
```

### prompts/get

根据 prompt 名称和参数生成消息。Agent 可直接使用生成的 messages 发起对话。

| Prompt | 用途 | 典型场景 |
|--------|------|----------|
| `analyze_build_failure` | 分析构建失败原因 | `cargo build` 失败后自动诊断 |
| `diagnose_test_failure` | 诊断测试失败 | 测试 task 完成但 exit_code ≠ 0 |
| `review_task_output` | 汇总任务输出 | 审查长命令的结构化事件 |
