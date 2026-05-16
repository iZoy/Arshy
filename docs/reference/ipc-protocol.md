# IPC 协议

## 传输

- 传输层：Unix Domain Socket
- 帧格式：JSON Lines（每行一个 JSON 对象）
- 编码：UTF-8
- 超时：60 秒（单次请求）

## 方法

| 方法 | 说明 | 参数 |
|------|------|------|
| `task/run` | 执行命令 | `RunTaskParams` |
| `task/query` | 查询事件 | `QueryParams` |
| `task/list` | 列出任务 | `{ status?, limit }` |
| `task/kill` | 终止任务 | `{ task_id }` |
| `task/tail` | 查看输出 | `{ task_id, lines, format }` |
| `task/stdin` | 写入 stdin（预留） | — |
| `daemon/status` | daemon 状态 | `{}` |
| `daemon/prune` | 清理数据 | `{ keep?, older_than? }` |
| `daemon/shutdown` | 优雅关闭 | `{}` |
| `daemon/stats` | 聚合统计 | `{}` |

## 通知

| 通知 | 说明 |
|------|------|
| `task/update` | 任务状态变更（running/completed/failed） |
| `task/complete` | 任务完成（含 exit_code） |
| `diagnostic` | 解析器诊断事件 |
| `daemon/shutdown` | daemon 正在关闭 |

## 数据类型

### RunTaskParams

```json
{
  "command": "cargo build",
  "cwd": "/home/user/project",
  "timeout_ms": 60000,
  "mode": "auto",
  "parse_hint": "json"
}
```

### RunTaskResponse

```json
{
  "task_id": "uuid",
  "status": "running",
  "pid": 12345
}
```

短命令（auto 模式）直接返回完整结果：

```json
{
  "task_id": "uuid",
  "status": "completed",
  "exit_code": 0,
  "duration_ms": 42,
  "raw_output": "hello\n",
  "short_command": true
}
```

### Task

```json
{
  "task_id": "uuid",
  "command": "cargo build",
  "cwd": "/home/user/project",
  "status": "completed",
  "exit_code": 0,
  "pid": 12345,
  "parser_name": "cargo",
  "started_at": "2026-01-01T00:00:00Z",
  "finished_at": "2026-01-01T00:00:05Z",
  "duration_ms": 5000,
  "events_count": 42,
  "error_count": 3
}
```

### TaskEvent

```json
{
  "seq": 1,
  "type": "diagnostic",
  "severity": "error",
  "code": "E0308",
  "message": "mismatched types",
  "location": {
    "file": "src/main.rs",
    "line": 42,
    "column": 5
  },
  "context": {
    "before": ["fn main() {"],
    "line": "    let x: i32 = \"hello\";",
    "after": ["}"]
  }
}
```

### QueryParams

```json
{
  "task_id": "uuid",
  "event_type": "diagnostic",
  "severity": "error",
  "code": "E0308",
  "file": "src/main.rs",
  "limit": 20,
  "offset": 0
}
```

### StatsResponse

```json
{
  "total_tasks": 150,
  "by_status": {
    "running": 2,
    "completed": 140,
    "failed": 5,
    "killed": 2,
    "timeout": 1
  },
  "total_events": 3000,
  "total_errors": 150,
  "avg_duration_ms": 2500.5,
  "p50_duration_ms": 800,
  "p99_duration_ms": 45000,
  "failure_rate": 0.033,
  "db_size_bytes": 1048576
}
```

## 数据库 Schema

### tasks

| 列 | 类型 | 说明 |
|----|------|------|
| `task_id` | TEXT PK | UUID |
| `command` | TEXT | 命令 |
| `cwd` | TEXT | 工作目录 |
| `status` | TEXT | running/completed/failed/killed/timeout |
| `exit_code` | INTEGER | 退出码 |
| `pid` | INTEGER | 进程 ID |
| `parser_name` | TEXT | 匹配的 parser |
| `started_at` | TEXT | ISO 时间戳 |
| `finished_at` | TEXT | ISO 时间戳 |
| `duration_ms` | INTEGER | 耗时 |
| `events_count` | INTEGER | 事件数 |
| `error_count` | INTEGER | 错误数 |

### events

| 列 | 类型 | 说明 |
|----|------|------|
| `id` | INTEGER PK | 自增 |
| `task_id` | TEXT FK | 关联任务 |
| `seq` | INTEGER | 序号 |
| `type` | TEXT | 事件类型 |
| `severity` | TEXT | error/warning/info |
| `code` | TEXT | 错误码 |
| `message` | TEXT | 消息 |
| `payload` | TEXT | 原始 JSON |
| `created_at` | TEXT | 时间戳 |

### tool_versions

| 列 | 类型 | 说明 |
|----|------|------|
| `tool_name` | TEXT PK | 工具名 |
| `version` | TEXT | 版本号 |
| `detected_at` | TEXT | 检测时间 |
| `expires_at` | TEXT | 缓存过期 |

### parser_registry

| 列 | 类型 | 说明 |
|----|------|------|
| `parser_name` | TEXT PK | Parser 名 |
| `tool_name` | TEXT | 工具名 |
| `parser_type` | TEXT | toml/rhai/raw |
| `source` | TEXT | builtin/user |
| `loaded_at` | TEXT | 加载时间 |
