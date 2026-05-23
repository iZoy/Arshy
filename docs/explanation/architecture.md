# 系统架构

## 组件总览

```
┌─────────────┐     stdio/MCP      ┌──────────────┐     UDS/IPC      ┌──────────────┐
│  MCP Client │ ◄────────────────► │  arshy proxy │ ◄──────────────► │  arshyd      │
│ (Claude Code│   1 call = 1 result│  (--from-mcp)│                  │  (daemon)    │
│  Cursor)    │                    └──────────────┘                  └──────┬───────┘
└─────────────┘                                                             │
                                                           ┌────────────────┼────────────────┐
                                                           │                │                │
                                                      ┌────▼────┐    ┌─────▼─────┐   ┌──────▼──────┐
                                                      │Executor │    │ Parser    │   │ Store       │
                                                      │(PTY)    │    │ Engine    │   │ (SQLite)    │
                                                      └────┬────┘    └───────────┘   └─────────────┘
                                                           │
                                                      ┌────▼────┐
                                                      │Process  │
                                                      │(进程组)  │
                                                      └─────────┘
```

## 三个进程

### arshy（proxy / CLI）

入口：`src/main.rs`

两种运行模式：
- `--from-mcp`：MCP stdio proxy，连接 MCP 客户端
- 子命令：CLI 直接调用（`arshy run`、`arshy list`、`arshy status` 等）

两种模式最终都通过 Unix Domain Socket（权限 0600）与 daemon 通信。

### arshyd（daemon）

入口：`src/daemon/main.rs`

后台常驻进程，职责：
1. 监听 UDS 连接（`Semaphore(64)` 速率限制）
2. 执行命令（Executor，PTY 分配）
3. 解析输出（Parser Engine，5 级管道）
4. 持久化数据（Store，SQLite WAL）
5. 安全过滤（Security，命令黑名单 + 沙箱路径 + 审计日志）
6. 遥测计数（Telemetry，原子计数器）
7. Panic 诊断（panic hook 记录堆栈）

生命周期：
```
启动 → panic hook → 写 PID → 检查 socket → 绑定 UDS → chmod 0o600 → accept 循环
                                                                              ↓ (ctrl-c/daemon/shutdown)
                                                              通知 shutdown → 移除 socket
                                                           → 5s 自然等待 → kill_all（进程组信号）
                                                           → 30s 硬截止 → 清理 PID → 退出
```

### Agent（外部）

Claude Code、Cursor 等 MCP 客户端。通过 stdio 与 proxy 通信，从 MCP `instructions` 中获知 arshy 是主 shell。

## 数据流

### 命令执行（Smart Sync）

```
Agent → arshy_exec(command: "cargo test")
         │
         ├─ 短命令？ → 瞬间返回原文
         │
         └─ 长命令 → Smart Sync（60s 超时）
              │
              ├─ Security check
              ├─ Parser detection
              ├─ PTY spawn
              ├─ 5 级 Parser 管道
              ├─ 事件存储到 SQLite
              ├─ 计算 summary + root_cause + project_context
              │
              └─ 返回完整结果（1 次 MCP 调用）
                   {status, exit_code, summary, root_cause, project_context, events[]}
```

### Parser 管道（5 级）

每行输出按优先级依次尝试，命中即停止：

```
行 → 格式检测(JSON/NDJSON/YAML/CSV) → Stateful(Rhai) → TOML(regex) → Crash(通用) → Raw
```

第一级命中则跳过后续级别。Pattern 编译时经过 ReDoS 安全校验。

### 智能输出

命令完成后自动计算：

| 字段 | 说明 |
|------|------|
| **summary** | `{by_type: {test_result: 31}, by_severity: {error: 0, warning: 2, info: 36}}` |
| **root_cause** | 第一个 error 级别事件（含 code/message/location） |
| **project_context** | 失败时附加 `git diff --stat HEAD~1` |

## 模块依赖

```
arshy_lib (crate: lib)
├── config     — 配置加载、env 扩展、log_level 校验、0 值防护、版本检查
├── error      — ArshyError 枚举、json_rpc_code 映射、is_retryable 判断
├── ipc        — JSON-RPC 类型、UDS 传输、错误码
└── mcp        — MCP 协议类型、工具/资源/指令定义

arshy (crate: bin, proxy)
├── cli        — clap 子命令分发
└── proxy      — MCP stdio 代理、health check 指数退避、reconnect 重试

arshyd (crate: bin, daemon)
├── bus        — EventBus 广播
├── context    — 连接上下文
├── exec       — 命令执行引擎
│   ├── process — 进程管理（进程组信号、优雅终止）
│   └── pty     — PTY 分配
├── ipc_handler — IPC 请求分发（cwd 继承、version 兼容检查）
├── lifecycle  — PID 文件、信号、socket 清理
├── parser     — 解析引擎
│   ├── toml_def — TOML schema v1.0 格式定义
│   ├── toml     — 无状态逐行匹配
│   ├── rhai     — 有状态脚本引擎
│   ├── crash    — 5 语言通用崩溃检测
│   ├── json     — 格式检测（JSON/NDJSON/YAML/CSV）
│   ├── redos    — ReDoS 安全校验
│   ├── registry — Parser 注册表 + 链式命令分段检测
│   ├── detect   — 工具检测、版本探测
│   └── loader   — 文件系统热重载
├── security   — 命令过滤、沙箱路径、审计日志
├── store      — SQLite WAL、schema 迁移
└── telemetry  — 原子计数器、stats/health 暴露
```

## 并发模型

- **单线程 tokio runtime**（proxy）：无需跨线程同步
- **多线程 tokio runtime**（daemon）：每个连接一个 spawn，Semaphore(64) 限制并发
- **事件总线**：`tokio::sync::broadcast`，多订阅者无阻塞
- **Parser 热重载**：`RwLock<ParserRegistry>`，读锁无阻塞，写锁短暂持有

## 存储

SQLite WAL 模式：

| 表 | 用途 | 索引 |
|----|------|------|
| `tasks` | 任务元数据 | `started_at`, `status` |
| `events` | 结构化事件 | `(task_id, seq)`, `(task_id, type, severity)` |
| `tool_versions` | 工具版本缓存 | — |
| `schema_version` | 迁移版本追踪 | — |
