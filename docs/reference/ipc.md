# Daemon IPC 参考

> 本文档依据 `src/ipc/mod.rs`、`src/ipc/transport.rs`、`src/daemon/ipc_handler.rs` 核对（arshy v0.1.0-alpha.1）。

## 传输

- 协议：JSON-RPC 2.0 over Unix Domain Socket（UDS），JSON Lines 帧（每行一个 JSON 对象，`\n` 结尾）；
- socket 路径：`daemon.socket_path`（默认 `${XDG_DATA_HOME}/arshy/arshyd.sock`），启动时绑定并设置为 0o600；
- 访问控制：daemon 拒绝不同 UID 的 peer 连接；并发连接上限 64（Semaphore）；
- 简单请求/响应：`transport::send_request` 发送一帧并读取匹配 `id` 的响应（跳过无 `id` 的通知帧）；
- 持久连接（`DaemonConnection`，proxy 使用）：写入队列 64、通知通道 256；请求默认 60s 超时（`send_request`），可自定义（`send_request_with_timeout`）；超时后清理 pending 项并返回 `request '<method>' timed out after <n>s`；
- 通知通道：proxy 侧 `DaemonConnection` reader 对通知通道（256）用阻塞发送——proxy 消费慢时 reader 阻塞、socket 停止读取、daemon 端写入随之减速；daemon 侧到连接的 outbound 通道（1024）用 `try_send`，满则丢弃并告警 `notification dropped (channel full or closed)`；
- 帧路由：有 `id` 且无 `method` → 响应；有 `method` → 通知。

## 帧格式

请求：

```json
{"jsonrpc":"2.0","id":1,"method":"task/run","params":{...}}
```

响应（成功）：

```json
{"jsonrpc":"2.0","id":1,"result":{...}}
```

错误响应（注意：daemon 把错误**包装在 `result` 内**的 `ErrorResponse` 对象中返回，而非 JSON-RPC 顶层 `error` 字段）：

```json
{"jsonrpc":"2.0","id":1,"result":{"jsonrpc":"2.0","id":1,"error":{"code":-32002,"message":"...","data":{"retryable":true,"command":"...","task_id":"..."}}}}
```

`error.data` 附加字段：`retryable`（bool，恒有）+ `command`/`task_id`（请求参数中存在时）。

通知：

```json
{"jsonrpc":"2.0","method":"task/update","params":{...}}
```

## 方法表

方法常量定义于 `src/ipc/mod.rs`（14 个）。

| 方法 | 参数 | 响应 result | 说明 |
|---|---|---|---|
| `task/run` | `RunTaskParams`（见下） | `RunResult` + 内联 `events`/`event_count`（见下） | 执行命令；`read-only` 下拒绝 |
| `task/query` | `QueryParams`（见下） | `{"events": [...], "total": n}` | 事件查询；`task_id` 有值 → 单任务；缺省 → 跨任务搜索 |
| `task/list` | `{"status": "string|无", "limit": u64}`（limit 默认 10） | `Task[]` | 按 `started_at` 倒序；`status_matches` 对引号容忍 |
| `task/kill` | `{"task_id": "string"}`（缺失 → INVALID_PARAMS） | `{"task_id": ..., "status":"killed"}` | 终止任务；`read-only` 下拒绝 |
| `task/tail` | `{"task_id": "string", "lines": u64(默认50, 0=全部), "format": "event\|raw"}` | `{"task_id": ..., "lines": ["..."]}` | 最近事件消息；`format=raw` 仅对持久化任务读取 `<store>/raw/<task_id>.txt` 捕获文本；无输出且无文件时返回空数组，预期输出文件缺失或读取失败时返回错误 |
| `daemon/status` | `{}` | `{"uptime_secs", "tasks_running", "tasks_total", "db_size_bytes", "parser_count", "counters"}` | 运行状态 + 加载 parser 数 + 遥测快照 |
| `daemon/health` | `{}` | `{"status":"ok|degraded","store_ok":bool,"store_integrity":"ok|degraded","store_integrity_error":string|null,"tasks_running","tasks_total","uptime_secs","counters"}` | 深度健康检查（store 可用性与 JSONL 完整性；损坏详情包含路径和行号） |
| `daemon/prune` | `{"keep": u64}` 或 `{"older_than": u64(天)}` 或 `{}` | `{"tasks_deleted": n, "events_deleted": n}` | keep → `prune_keep`；older_than → `prune_older_than`；都无 → `prune_keep(1000)`；`read-only` access level 不拦截，调用会删除保留历史 |
| `daemon/shutdown` | `{}` | `{"status":"shutting_down"}` | 触发 watch 关闭信号；本机同 UID 的 IPC 调用不受 `read-only` access level 限制 |
| `daemon/stats` | `{}` | `StatsResponse` | 聚合统计（JSONL store；`db_size_bytes` 为 store 文件总大小） |
| `daemon/analyze` | `{}` | `ImpactReport` | 影响分析报告（阻塞任务内 spawn_blocking） |
| `session/cd` | `{"command": "path"}` | `{"cwd": "<绝对路径>"}` | canonicalize 后设置该连接会话 cwd；后续 `task/run` 未带 `cwd` 时自动注入 |
| `task/subscribe` | `{"task_id": "string"}` | 任务已终态：`{"task_id","status","exit_code","duration_ms"}`；否则等待 `task/complete` 后返回同结构（`status` 为 `completed`/`failed`） | 等待任务完成；600s 超时 → `subscribe <id> timed out after 600s` |
| `parser/reload` | `{}` | `{"diff": "string"}` | 从磁盘重载 parser 注册表并返回 diff 文本 |

`counters` 结构（`src/daemon/telemetry.rs`）：`connections_accepted`、`events_emitted`、`tasks_created`、`tasks_completed`、`tasks_failed`。

## RunTaskParams（task/run）

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `command` | string | 必填 | 要执行的命令 |
| `cwd` | string | 无（会话 cwd 注入） | 工作目录 |
| `timeout_ms` | u64 | 无 | 任务超时（executor 强制上限 `max_task_duration_ms`） |
| `mode` | string | `auto` | `auto`/`sync`/`async` |
| `parse_hint` | string | 无 | 强制指定 parser（如 `python`、`cargo`、`raw`）；JSON 输出由管线自动检测 |
| `env` | object | 无 | 环境变量，继承父进程并覆盖/追加 |
| `errors_only` | bool | `false` | 仅保留 error 级事件 |
| `purpose` | string | 无 | 用途标签（`dogfood` 等）；缺省/`real` 计为真实开发负载 |

### mode 语义（`src/daemon/exec/mod.rs`）

- `auto` + 原始快速路径（只读检查命令且无 parse_hint）→ 同步执行，返回捕获文本（兼容字段 `short_command: true`）；fast-path task ID 不写入 Store，不能用于后续 query/tail；
- `auto` + 结构化路径（parser 命中或存在生命周期信号）→ 同步等待，60s 内未完成则降级为 async（返回 `status:"running"` 的 `RunResult`）；
- `sync` → 显式同步，无限等待（由 `timeout_ms` 控制）；
- `async` → 立即返回任务 ID，事件经通知流推送。

`is_short_command` 不再维护生态命令前缀表。parser registry 命中即选择结构化路径；只读检查工具保留原始快速返回。Rust 规则只处理 shell 组合、后台/重定向、watch/server 信号和无 parser 时的字数/长度兜底。

## RunResult（task/run 响应）

| 字段 | 类型 | 说明 |
|---|---|---|
| `task_id` | string | IPC 始终带任务 ID；MCP 只在运行中或输出不完整时展示 |
| `status` | enum | `running`/`completed`/`failed`/`killed`/`timeout`；表示任务生命周期结果 |
| `exit_code` | i32 or null | 进程退出码；运行中为 null。`completed` 表示 0，普通非零退出为 `failed`。服务终止时使用负哨兵：`-1` 表示没有数值进程码（如信号终止/worker 恢复），`-2` 表示达到执行超时，`-3` 表示后台 worker 完成了 arshy 取消；判断结果应优先看 `status` |
| `duration_ms` | u64 | 完成时存在 |
| `error_count` | u64 | error 事件数 |
| `warning_count` | u64 | warning 事件数 |
| `raw_output` | string | 当前捕获文本；经 UTF-8 替换和逐行处理，不是字节级原始输出 |
| `short_command` | bool | 是否走了零开销快路径 |
| `primary_diagnostic` | value | 从完整错误事件集合选择的代表诊断，不宣称因果 |
| `events` | TaskEvent[] | 内联事件：失败 ≤20 条 error、成功 ≤5 条 warning/info（daemon 追加） |
| `event_count` | u64 | 该任务事件总数（daemon 追加） |
| `events_truncated` | bool | 内联事件被截断时（daemon 追加） |
| `events_hint` | string | 提示用 `arshy_query` 获取完整事件（daemon 追加） |

### 输出保留与截断

默认 `daemon.max_output_bytes` 为 10 MiB；两个执行路径对 stdout/stderr
合计计数，超过上限后停止捕获后续文本但继续排空管道。结构化任务会发出
截断事件并持久化截断标记；快路径只在返回文本中放标记。单行最多保留
64 KiB，超长行会附截断标记；非法 UTF-8 会以替换字符呈现。未以换行结尾
的最后一行会保留为一行。短命令一旦总量截断，会将已捕获前缀和任务记录
落盘；超过捕获上限的字节已经丢弃。`raw_output_bytes` 统计 reader 实际读取的源字节，
包括超出捕获上限后为排空而读取的字节，因此可大于保留文本大小；超时只
统计截止前读取的部分。捕获文本不保证字节级重放，两个 reader 之间的相对
顺序也不保证。

结构化 `auto` 模式在 60 秒后只停止同步等待并返回 `running` 任务；后台任务
仍按 `timeout_ms` 与 daemon 的 `max_task_duration_ms` 上限继续运行。显式
`sync` 等待到任务终止，除非设置的执行 timeout 或 `max_task_duration_ms`
上限触发。两者都不把 auto 的 60 秒交接时间当作执行超时。

## QueryParams（task/query）

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `task_id` | string | 无 | 有值 → 单任务查询；缺省 → 跨任务搜索（要求 IPC 层直接调用） |
| `event_type` | string | 无 | 精确匹配事件 `type` |
| `severity` | string | 无 | 精确匹配 `severity` |
| `code` | string | 无 | 精确匹配 `code` |
| `file` | string | 无 | `location.file` 子串匹配 |
| `limit` | usize | `20` | 分页大小 |
| `offset` | usize | `0` | 分页偏移 |
| `include_logs` | bool | `false` | 为 true 时包含 `log` 类型事件；默认排除 |

跨任务搜索（`store.search_events`）：按事件文件 mtime 新→旧，每个事件注入 `task_id` 字段；`total` 为过滤后总数。

## 数据类型

### Task

| 字段 | 类型 | 说明 |
|---|---|---|
| `task_id` | string | 任务 ID |
| `command` | string | 命令 |
| `cwd` | string | 工作目录（可选） |
| `status` | enum | `running`/`completed`/`failed`/`killed`/`timeout`（snake_case 序列化） |
| `exit_code` | i32 | 可选 |
| `pid` | u32 | 可选 |
| `parser_name` | string | 命中的 parser 名（如 `cargo`、`raw`），可选 |
| `started_at` | string | RFC3339 |
| `finished_at` | string | 可选 |
| `duration_ms` | u64 | 可选 |
| `events_count` | u64 | 事件数 |
| `error_count` | u64 | error 事件数 |
| `purpose` | string | 用途标签（可选） |

终态：`completed`/`failed`/`killed`/`timeout`（`is_terminal`）。

### TaskEvent

| 字段 | 类型 | 说明 |
|---|---|---|
| `seq` | u64 | 行序号 |
| `type` | string | 事件类型（`diagnostic`、`location`、`test_result`、`summary`、`crash`、`data`、`log` 等） |
| `severity` | string | `error`/`warning`/`info`（可选） |
| `code` | string | 错误码（可选） |
| `message` | string | 消息文本 |
| `location` | object | `{"file","line","column?"}`（可选） |
| `context` | object | 旧存储字段；打开存储时迁移删除，当前响应不序列化此字段 |
| `hint` | object | `{"cause","fix?","retry?"}`；保留为空兼容占位（HintDb 已移除，`cause`/`fix` 不再生成） |

### StatsResponse

`total_tasks`、`by_status`、`total_events`、`total_errors`、`purpose_breakdown`、`avg_duration_ms`、`p50_duration_ms`、`p99_duration_ms`、`failure_rate`、`db_size_bytes`、`parser_coverage_pct`、`dedup_collapsed`、`per_parser_usage`、`total_raw_output_bytes`、`total_structured_output_bytes`、`total_visible_events`、`total_skipped_noise_events`、`total_locations_extracted`、`total_codes_extracted`、`efficiency`。可选字段缺省时省略（`skip_serializing_if`）。`efficiency` 使用 `quality-v2` 分项指标；diagnostic completeness 现为 unavailable，不包含 token 估算。

## 通知（daemon → proxy）

`src/daemon/bus/router.rs` `NotificationRouter` 将 EventBus 事件映射为 IPC 通知：

| 方法 | params | 来源事件 |
|---|---|---|
| `task/update` | `{"task_id","status","elapsed_ms"}` | 任务状态更新 |
| `task/complete` | `{"task_id","exit_code","duration_ms"}` | 任务完成 |
| `diagnostic` | `{"task_id","event":<TaskEvent>}` | 结构化诊断事件 |
| `daemon/shutdown` | `{"reason","grace_period_ms"}` | daemon 关闭广播 |

`StreamOutput` 事件为保留类型（实时输出流，当前任何代码路径都不产生）。

## 错误码表

标准 JSON-RPC 2.0（`src/ipc/mod.rs` error_code）：

| 码 | 常量 | 产生条件 |
|---|---|---|
| `-32700` | `PARSE_ERROR` | 请求 JSON 解析失败（`id` 固定 0） |
| `-32600` | `INVALID_REQUEST` | 保留（当前无产生路径） |
| `-32601` | `METHOD_NOT_FOUND` | IPC 消息含 `unknown method` |
| `-32602` | `INVALID_PARAMS` | 消息含 `missing` 或 `invalid`（如缺 `task_id`、URI 非法、参数反序列化失败） |
| `-32603` | `INTERNAL_ERROR` | 其他所有错误（含命令被黑名单拦截） |

应用错误码：

| 码 | 常量 | 产生条件 |
|---|---|---|
| `-32001` | `TASK_NOT_FOUND` | `TaskNotFound` |
| `-32002` | `TASK_TIMEOUT` | `TaskTimeout` |
| `-32003` | `ACCESS_DENIED` | `AccessDenied`（read-only 模式下 run/kill） |
| `-32004` | `COMMAND_BLOCKED` | 命令被安全过滤器/白名单拦截时产生（`ArshyError::Blocked`）；不可重试 |
| `-32005` | `RATE_LIMITED` | IPC 消息含 `rate limit`（限流触发） |

### retryable 语义（`src/error.rs` is_retryable）

| 错误 | retryable |
|---|---|
| `TaskTimeout` | true |
| `DaemonUnreachable` | true |
| IPC 消息含 `timed out` 或 `connection closed` | true |
| `Io` | true |
| 其他（TaskNotFound、AccessDenied、Config、未知方法等） | false |

## 与直觉不符的事实（代码核对）

1. **错误响应嵌套**：daemon 将 `ErrorResponse`（含 `error.code`）包装在成功 JSON-RPC 响应的 `result` 字段中返回，符合 JSON-RPC 的顶层 `error` 结构并未使用；调用方必须检查 `result.error`。
2. `session/cd` 的会话 cwd 是**按连接**存储的（`handle` 循环内的 `default_cwd`），不是全局/按进程的。
3. `task/subscribe` 的 600s 硬超时与 proxy 的 60s 请求超时不同层；MCP 客户端订阅通常先遇到 proxy 侧超时。
