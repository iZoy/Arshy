# IPC 协议

## 传输

- 传输层：Unix Domain Socket（权限 0600）
- 帧格式：JSON Lines（每行一个 JSON 对象）
- 编码：UTF-8
- 超时：60 秒（单次请求）
- 速率限制：`Semaphore(64)` 并发连接

## 方法

| 方法 | 说明 | 参数 |
|------|------|------|
| `task/run` | 执行命令 | `RunTaskParams` |
| `task/query` | 查询结构化事件 | `QueryParams` |
| `task/list` | 列出任务 | `{ status?, limit }` |
| `task/kill` | 终止任务（进程组信号） | `{ task_id }` |
| `task/tail` | 查看任务输出 | `{ task_id, lines, format }` |
| `task/subscribe` | 阻塞等待任务完成 | `{ task_id }` |
| `session/cd` | 设置会话工作目录 | `{ command: "/path" }` |
| `daemon/status` | 运行状态 + 遥测 | `{}` |
| `daemon/health` | 深度健康检查 + 遥测 | `{}` |
| `daemon/stats` | 聚合统计 + 遥测 | `{}` |
| `daemon/prune` | 清理历史数据 | `{ keep?, older_than? }` |
| `daemon/shutdown` | 优雅关闭 | `{}` |

## 数据类型

### RunTaskParams

| 字段 | 类型 | 说明 |
|------|------|------|
| `command` | string | Shell 命令 |
| `cwd` | string? | 工作目录 |
| `timeout_ms` | u64? | 超时 |
| `mode` | string | auto（默认）/ sync / async |
| `parse_hint` | string? | 输出格式提示或 parser 名称 |
| `env` | map? | 环境变量 |

### RunResult

| 字段 | 类型 | 说明 |
|------|------|------|
| `task_id` | string | 任务 ID |
| `status` | enum | running / completed / failed / killed / timeout |
| `pid` | u32? | 进程 PID |
| `exit_code` | i32? | 退出码 |
| `duration_ms` | u64? | 执行耗时 |
| `event_count` | u64? | 事件总数 |
| `error_count` | u64? | 错误事件数 |
| `raw_output` | string? | 短命令原始输出 |
| `short_command` | bool | 是否走了短路径 |

### QueryParams

| 字段 | 类型 | 说明 |
|------|------|------|
| `task_id` | string | 任务 ID |
| `event_type` | string? | 事件类型过滤 |
| `severity` | string? | 严重度过滤 |
| `code` | string? | 错误码过滤 |
| `file` | string? | 文件路径过滤 |
| `limit` | usize | 最大条数（默认 20） |
| `offset` | usize | 偏移（分页） |

### TaskEvent

| 字段 | 类型 | 说明 |
|------|------|------|
| `seq` | u64 | 序列号 |
| `event_type` | string | diagnostic / location / test_result / crash / summary / log |
| `severity` | string? | error / warning / info |
| `code` | string? | 错误码 |
| `message` | string | 消息文本 |
| `location` | object? | `{ file, line, column }` |
| `context` | object? | `{ line, before[], after[] }` |

## 通知

| 通知 | 说明 |
|------|------|
| `task/update` | 任务状态变更（running/completed/failed） |
| `task/complete` | 任务完成（含 exit_code、duration_ms） |
| `diagnostic` | 结构化诊断事件（含 event_type/severity/code/location） |
| `daemon/shutdown` | Daemon 即将关闭 |

## 错误码

使用 JSON-RPC 2.0 错误码。详见 [错误码参考](error-codes.md)。
