# 错误码参考

## JSON-RPC 错误码

Arshy 使用 JSON-RPC 2.0 错误码体系。

### 标准错误码

| 码 | 名称 | 说明 | 可重试 |
|----|------|------|--------|
| `-32700` | Parse Error | JSON 解析失败 | ❌ |
| `-32600` | Invalid Request | 请求格式无效 | ❌ |
| `-32601` | Method Not Found | 方法不存在 | ❌ |
| `-32602` | Invalid Params | 参数无效 | ❌ |
| `-32603` | Internal Error | 内部错误 | ❌ |

### 应用错误码

| 码 | 名称 | 说明 | 可重试 |
|----|------|------|--------|
| `-32001` | Task Not Found | 任务 ID 不存在 | ❌ |
| `-32002` | Task Timeout | 任务超时 | ✅ |
| `-32003` | Access Denied | 安全策略拒绝 | ❌ |
| `-32004` | Command Blocked | 命令被黑名单拦截 | ❌ |

## 错误类型

| ArshyError 变体 | 映射到 | 说明 |
|----------------|--------|------|
| `TaskNotFound` | `-32001` | 查询不存在的任务 |
| `TaskTimeout` | `-32002` | 超过 `max_task_duration_ms` |
| `AccessDenied` | `-32003` | 只读模式或路径受限 |
| `Ipc("unknown method: ...")` | `-32601` | 未注册的 IPC 方法 |
| `Ipc("missing ..." / "invalid ...")` | `-32602` | 参数缺失或类型错误 |
| `TaskExecution` | `-32603` | 进程退出码非零 |
| `Parser` | `-32603` | Parser 解析失败 |
| `Sqlite` | `-32603` | 数据库错误 |
| `Exec` | `-32603` | 进程 spawn/exec 失败 |
| `Mcp` | `-32603` | MCP 协议错误 |
| `Serialization` | `-32603` | JSON/TOML 序列化失败 |
| `Config` | `-32603` | 配置文件错误 |
| `Io` | `-32603` | 系统 I/O 错误 |
| `DaemonUnreachable` | `-32603` | Daemon 无法连接 |
| `Other` | `-32603` | 未分类内部错误 |

## 重试语义

可重试错误：

- `TaskTimeout` — 增加 `timeout_ms` 后重试
- `DaemonUnreachable` — 等待 daemon 恢复后重试
- `Ipc("timed out" | "connection closed")` — 连接问题，可重试
- `Io` — 系统 I/O 错误，通常可重试

不可重试错误：

- `TaskNotFound` — 检查 task_id 是否正确
- `AccessDenied` — 检查安全配置
- `Config` — 修复配置
- `Ipc("unknown method")` — 检查请求格式

## MCP 层错误

MCP 层通过标准 MCP error response 返回错误：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32001,
    "message": "task not found: abc-123"
  }
}
```

IPC 层（daemon ↔ proxy）使用相同错误码体系。
