# 错误码参考

## JSON-RPC 错误码

Arshy 使用 JSON-RPC 2.0 错误码体系。**所有 MCP error 响应统一携带 `data.retryable` 字段**，agent 可据此自动决策是否重试。

### 标准错误码

| 码 | 名称 | 说明 | retryable |
|----|------|------|:--------:|
| `-32700` | Parse Error | JSON 解析失败 | false |
| `-32600` | Invalid Request | 请求格式无效 | false |
| `-32601` | Method Not Found | 方法不存在 | false |
| `-32602` | Invalid Params | 参数无效 | false |
| `-32603` | Internal Error | 内部错误 | false |

### 应用错误码

| 码 | 名称 | 说明 | retryable |
|----|------|------|:--------:|
| `-32001` | Task Not Found | 任务 ID 不存在 | false |
| `-32002` | Task Timeout | 任务超时 | true |
| `-32003` | Access Denied | 安全策略拒绝 | false |
| `-32004` | Command Blocked | 命令被黑名单拦截 | false |

## 错误类型

| ArshyError 变体 | JSON-RPC 码 | retryable | 说明 |
|----------------|-------------|:---------:|------|
| `TaskNotFound` | `-32001` | false | 查询不存在的任务 |
| `TaskTimeout` | `-32002` | true | 超过 max_task_duration_ms |
| `AccessDenied` | `-32003` | false | 只读模式或路径受限 |
| `CommandBlocked` | `-32004` | false | 命令被黑名单拦截 |
| `Ipc("unknown method")` | `-32601` | false | 未注册的 IPC 方法 |
| `Ipc("missing/invalid...")` | `-32602` | false | 参数缺失或类型错误 |
| `DaemonUnreachable` | `-32603` | true | Daemon 无法连接，proxy 自动重连 |
| `Ipc("timed out")` | `-32603` | true | 请求超时，可增加 timeout_ms 重试 |
| `Ipc("connection closed")` | `-32603` | true | 连接断开，proxy 自动重连 |
| `TaskExecution` | `-32603` | false | 进程退出码非零（非 arshy 自身错误） |
| `Io` | `-32603` | true | 系统 I/O 错误 |
| `Config` | `-32603` | false | 配置文件错误，修复配置 |
| `Parser/Exec/Sqlite/Mcp` | `-32603` | false | 内部错误 |

## MCP 层错误响应格式

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

`data.retryable` 在所有 MCP 错误路径上统一返回（包括 `write_json_error` 和 `write_structured_error`）。

## 重试策略

**可重试（retryable: true）**：
- `DaemonUnreachable` — proxy 已自动重连，等待后重试
- `TaskTimeout` — 增加 `timeout_ms` 后重试
- `Ipc("timed out" / "connection closed")` — 连接问题，可重试
- `Io` — 系统 I/O 错误

**不可重试（retryable: false）**：
- `TaskNotFound` — 检查 task_id 是否正确
- `AccessDenied` / `CommandBlocked` — 检查安全配置
- `Config` — 修复配置文件
- `Ipc("unknown method" / "invalid params")` — 检查请求格式
