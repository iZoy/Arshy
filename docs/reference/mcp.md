# MCP 协议参考

> 本文档依据 `src/mcp/instructions.rs`、`src/mcp/protocol.rs`、`src/proxy/mod.rs` 核对（arshy 0.2.0）。

## 传输与运行方式

MCP server 以 stdio 方式运行：`arshy --from-mcp`。stdin 读入 MCP JSON-RPC 2.0（每行一个 JSON），stdout 写出响应与通知；proxy 通过 UDS 与 `arshyd` daemon 通信，收到 SIGTERM 时向 daemon 转发 `daemon/shutdown` 后优雅退出（`src/proxy/mod.rs` proxy_main）。

启动时最多重试 5 次连接（间隔 500ms×4），每次成功连接后发送 `session/cd` 到当前目录验证连接存活（`connect_with_retry`，`validate_with_cd=true`）。

## initialize 与版本协商

客户端发送 `initialize`，服务端在 `handle_initialize` 中协商版本（`src/proxy/mod.rs`）：

- 支持版本列表（`SUPPORTED_PROTOCOL_VERSIONS`）：`2024-11-05`、`2025-03-26`、`2025-06-18`、`2025-11-25`；
- 客户端请求的 `protocolVersion` 若在列表中则**原样回显**；未知或缺失则回退默认 `2024-11-05`；
- 按 MCP 规范，版本不匹配时**不报错**，由客户端决定是否继续。

响应（`result`）：

| 字段 | 值 |
|---|---|
| `protocolVersion` | 协商结果（见上） |
| `serverInfo.name` | `arshy` |
| `serverInfo.version` | Cargo 包版本（`env!("CARGO_PKG_VERSION")`） |
| `capabilities.tools.listChanged` | `true` |
| `capabilities.resources.listChanged` | `true` |
| `capabilities.logging` | `{}` |
| `instructions` | 默认指令文本（`instructions::default_instructions()`） |

`instructions` 核心内容：所有命令经 `arshy_exec(action:"run", command:"<cmd>")` 执行（`DaemonUnreachable` 时允许一次性回退 Bash）；`mode:"auto"` 一次调用返回完整结果（短命令文本、长命令结构化诊断）；`arshy_exec(action:"cd", ...)` 设置会话目录；不管理 task ID、不轮询。

## 支持的请求方法

| 方法 | 处理 |
|---|---|
| `initialize` | 版本协商 + 能力声明（见上） |
| `notifications/initialized` | 忽略（`Ok(())`） |
| `ping` | 返回空 `result` `{}` |
| `tools/list` | 返回 2 个工具定义（见下） |
| `tools/call` | 映射到 IPC 方法并转发（见下） |
| `resources/list` | 最近 50 个任务作为资源（`task/list` limit 50） |
| `resources/read` | 按 `arshy://task/<id>` 读取任务事件 |
| `notifications/cancelled` | 若取消的是进行中的工具调用，kill 对应任务（`request_tasks` 映射） |
| 其他请求（带 `id`） | 返回 JSON-RPC error（`unknown method`） |
| 其他通知 | 静默忽略 |

## tools

### tools/list

两个工具（`src/mcp/instructions.rs` tool_definitions）：

| 工具 | 用途 |
|---|---|
| `arshy_exec` | 统一执行入口：`run`/`cd`/`kill`/`list`/`tail`/`raw`/`subscribe` 动作 |
| `arshy_query` | 查询结构化事件（单任务或跨任务搜索） |

### arshy_exec 参数 schema

```
type: object
required: ["action"]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `action` | string enum `["run","kill","list","tail","raw","cd","subscribe"]` | 必填 | 动作；proxy 映射时缺失/未知按 `run` 处理 |
| `command` | string | 无 | `run`：shell 命令；`cd`：目录路径 |
| `cwd` | string | 无 | 工作目录（一次性覆盖；session 级用 `action:cd`） |
| `timeout_ms` | integer | 无 | 超时毫秒数 |
| `mode` | string enum `["auto","sync","async"]` | `auto` | `auto`：智能区分短/长；`sync`：等待；`async`：立即返回 |
| `parse_hint` | string | 无 | 强制指定 parser（如 `python`、`cargo`、`raw`）；JSON 输出由管线自动检测，不存在 csv/table 提示 |
| `env` | object | 无 | 环境变量键值对（如 `{"RUST_LOG":"debug"}`） |
| `task_id` | string | 无 | `kill`/`tail`/`raw` 的目标任务 ID |
| `lines` | integer | `50` | `tail` 行数；`raw` 默认 `200`，`0` = 全部 |
| `format` | string enum `["event","raw"]` | `event` | `tail` 输出格式 |
| `status` | string enum `["running","completed","failed","killed"]` | 无 | `list` 状态过滤 |
| `limit` | integer | `10` | `list` 最大条数 |

### arshy_exec 动作 → IPC 方法映射

`src/proxy/mod.rs` `mcp_tool_to_ipc_method`：

| action | IPC 方法 |
|---|---|
| `kill` | `task/kill` |
| `list` | `task/list` |
| `tail` | `task/tail` |
| `raw` | `task/tail`（proxy 强制 `format=raw`，返回任务原始输出） |
| `cd` | `session/cd` |
| `subscribe` | `task/subscribe` |
| 其他（含 `run`、缺失） | `task/run` |

旧工具名仍被接受（向后兼容）：`arshy_run` → `task/run`、`arshy_kill` → `task/kill`、`arshy_list` → `task/list`、`arshy_tail` → `task/tail`；未知工具名默认 `task/run`。

### arshy_query 参数 schema

```
type: object
required: []
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `task_id` | string | 无 | 指定任务；省略则跨任务搜索（事件带 `task_id`） |
| `event_type` | string | 无 | 事件类型过滤（如 `compile_error`、`lint`） |
| `severity` | string enum `["error","warning","info"]` | 无 | 严重级别过滤 |
| `code` | string | 无 | 错误码过滤 |
| `file` | string | 无 | 文件路径过滤（子串） |
| `limit` | integer | `20` | 最大返回事件数 |

`arshy_query` 的 MCP 响应为 `{"content":[{"type":"text","text":"N events"}], "events": [...], "total": N}`（`build_query_result`）。

### tools/call 响应（arshy_exec 非 query 动作）

daemon 返回 IPC 错误对象（含 `error.code`）时，proxy 以 JSON-RPC error 返回（`data.retryable=false`）。

短命令（`short_command=true`，无 parse_hint 的 auto 短命令）：`content` 为原始输出文本；空输出且失败/超时时为 `[command timed out: exit code N]` / `[command failed: exit code N]`。

长命令：`content` 为一行摘要 `✓ N errors, 2.3s (exit 1)`（状态图标：`✓` completed、`✗` failed/timeout、`⊘` killed、`⟳` running、`?` 其他），失败时附加 `Root cause: ...` 与 `Changed files:`（git diff stat）。

`result` 顶层附加字段（长命令）：

| 字段 | 说明 |
|---|---|
| `content` | MCP content 数组（`type:"text"`） |
| `isError` | 见下"isError 判定" |
| `task_id`、`status`、`exit_code`、`duration_ms` | 任务元数据 |
| `error_count`、`warning_count` | 事件计数 |
| `root_cause` | 首个 error 级诊断事件 |
| `project_context` | 项目上下文（git 变更等） |
| `raw_output` | 原始输出（仅短命令路径由 daemon 返回；长命令恒缺省，用 `arshy_exec(action:"raw", task_id, lines)` 获取；零结构化事件时响应内容会提示该通道） |
| `events`、`event_count` | daemon 内联事件（失败 ≤20 条 error 事件，成功 ≤5 条 warning/info 事件）与总数；截断时含 `events_truncated`、`events_hint` |

### isError 判定

以下任一条件置 `isError=true`（`src/proxy/mod.rs`）：

- `status == "failed"` 或 `status == "timeout"`；
- `exit_code >= 2`（exit 1 视为歧义：grep 无匹配、diff 差异、测试条件为假，不置错）；
- `status == "completed"` 且 `error_count > 0`。

## resources

### resources/list

最近 50 个任务（`task/list` limit 50），每个任务一个资源：

| 字段 | 值 |
|---|---|
| `uri` | `arshy://task/<task_id>` |
| `name` | `<command> [<status>]` |
| `description` | `Task <task_id> — <command>` |
| `mimeType` | `application/json` |

### resources/read

URI 格式：`arshy://task/<task_id>`（其他 URI 返回 INVALID_PARAMS `invalid resource URI`）。

响应 `result.contents` 数组含单个条目：

| 字段 | 值 |
|---|---|
| `uri` | 请求的 URI |
| `mimeType` | `application/json` |
| `text` | pretty JSON：`{"task_id": ..., "total_events": N, "events": [...]}`（`task/query` limit 200） |

## notifications

daemon IPC 通知经 proxy 转为 MCP `notifications/message`（`write_mcp_notification`）：

| IPC 通知 | MCP level | logger | data 字段 |
|---|---|---|---|
| `task/update` | `info` | `arshy` | `{"event":"task_update","task_id":...,"status":...}` |
| `task/complete` | `info`（exit 0）/`error`（非 0） | `arshy` | `{"event":"task_complete","task_id":...,"exit_code":...}` |
| `diagnostic` | 按事件 severity：`error`→`error`、`warning`→`warning`、`debug`→`debug`、其他→`info` | `arshy.parser` | `{"event":"diagnostic","task_id":...,"type":...,"severity":...,"message":...,"location":...}` |
| `daemon/shutdown` | `warning` | `arshy.daemon` | `{"event":"shutdown","reason":...}` |
| 未知通知 | — | — | 跳过 |

### 通知批量

- 单条通知：直接发送 `notifications/message`；
- 多条通知在 `notifications.batch_interval_ms`（默认 100ms）窗口内合并；达到 `notifications.max_batch_events`（默认 50）立即 flush；
- 合并后为单条 `notifications/message`，`level:"info"`、`logger:"arshy.batch"`、`data:{"event":"batch","count":N,"items":[...]}`，items 元素为 `{"event":"task_update|task_complete|diagnostic|shutdown", ...}`。

## 错误响应

JSON-RPC error 结构（`write_structured_error` / `write_json_error`）：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32002,
    "message": "task timed out after 5000ms",
    "data": { "retryable": true }
  }
}
```

错误码沿用 IPC 错误码表（见 ipc.md）；`data.retryable` 由 `ArshyError::is_retryable()` 决定（TaskTimeout、DaemonUnreachable、连接超时/断开、IO 错误为 true）。

`DaemonUnreachable` 的典型消息提示运行 `arshy daemon start`；proxy 在连接错误且 `daemon.auto_start` 开启时先尝试按需拉起 daemon 并重试一次请求，失败后返回结构化错误。连续 5 次/2 分钟崩溃会触发熔断（`/tmp/arshyd.spawn-lock` + 崩溃日志），停止自动启动。

## 与直觉不符的事实（代码核对）

1. `arshy_exec` schema 的 `action` enum 含 `subscribe`（`task/subscribe` IPC 方法），但 `src/mcp/instructions.rs` 的描述文本只提到 run/cd/kill/list/tail/raw。
2. `arshy_query` 的 `task_id` 省略时执行跨任务搜索（`store.search_events`），结果事件对象注入 `task_id` 字段——这是 IPC/`TaskEvent` 结构本身没有的字段。
3. `tail` 的 `format` 参数（`event`/`raw`）在 daemon executor 层未使用。
4. 工具调用成功但 exit_code 为 1 时不置 `isError`（有意为之，但容易被误认为"成功"）。
5. 通知合并会改变事件到达粒度：多事件被包装为单条 `notifications/message`，且 `task/update` 只保留 `task_id`/`status`（丢弃 `elapsed_ms`）。
