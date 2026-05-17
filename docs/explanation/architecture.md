# 系统架构

## 组件总览

```
┌─────────────┐     stdio/MCP      ┌──────────────┐     UDS/IPC      ┌──────────────┐
│  MCP Client │ ◄──────────────►  │  arshy proxy │ ◄──────────────► │  arshyd      │
│ (Claude Code│                    │  (--from-mcp)│                  │  (daemon)    │
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
- `--from-mcp`：MCP stdio proxy，连接 MCP 客户端。初始化时宣告 `experimental.preferredShell` 能力和权威 `instructions`
- 子命令：CLI 直接调用（`arshy run`、`arshy list`、`arshy status` 等）

两种模式最终都通过 Unix Domain Socket（权限 0600）与 daemon 通信。

### arshyd（daemon）

入口：`src/daemon/main.rs`

后台常驻进程，职责：
1. 监听 UDS 连接（`Semaphore(64)` 速率限制）
2. 执行命令（Executor，PTY 分配）
3. 解析输出（Parser Engine，4 级管道）
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

Claude Code、Cursor 等 MCP 客户端。通过 stdio 与 proxy 通信，从 MCP `instructions` 和 `experimental.preferredShell` 能力中获知 arshy 是主 shell。

## 数据流

### 命令执行

```
Agent → MCP tool call → Proxy → IPC task/run → Daemon
                                                  │
                                          ┌───────▼────────┐
                                          │ Security check  │
                                          │ (blocked_patterns│
                                          │  allowed_commands│
                                          │  sandbox_paths)  │
                                          └───────┬─────────┘
                                                  │ pass
                                          ┌───────▼────────┐
                                          │ is_short_cmd?  │
                                          └──┬──────────┬──┘
                                             │yes       │no
                                      ┌──────▼──┐  ┌───▼──────────┐
                                      │直接执行  │  │创建任务      │
                                      │返回stdout│  │写入Store     │
                                      └─────────┘  │version probe │
                                                   │Parser session│
                                                   │PTY spawn     │
                                                   │逐行解析      │
                                                   │事件写入Store │
                                                   │通知推送Proxy │
                                                   └──────────────┘
```

### Parser 管道

每行输出经过四级匹配：

```
行 → Stateful Parser? → Line Patterns? → Crash Parser? → Raw
         │                    │               │            │
    (stateful toml       (toml 逐行      (通用崩溃      (原始文本
     或 rhai 脚本)         匹配)           检测)         log事件)
```

第一级命中则跳过后续级别。Pattern 编译时经过 ReDoS 安全校验（嵌套量词、重叠交替拒绝加载）。详见 [Parser 管线](parser-pipeline.md)。

### 通知流

```
Executor 事件 → EventBus → IPC Handler → Notification Channel
                                               │
                                        ┌──────▼──────┐
                                        │ Batch 汇聚   │
                                        │ (100ms窗口)  │
                                        └──────┬──────┘
                                               │
                                        ┌──────▼──────┐
                                        │ Proxy stdout │
                                        │ (MCP notif)  │
                                        └─────────────┘
```

## 模块依赖

```
arshy_lib (crate: lib)
├── config     — 配置加载、env 扩展、log_level 校验、0 值防护、版本检查
├── error      — ArshyError 枚举、json_rpc_code 映射、is_retryable 判断
├── ipc        — JSON-RPC 类型、UDS 传输、错误码
└── mcp        — MCP 协议类型、工具/资源/指令定义、experimental capabilities

arshy (crate: bin, proxy)
├── cli        — clap 子命令分发
└── proxy      — MCP stdio 代理、health check 指数退避、reconnect 重试

arshyd (crate: bin, daemon)
├── bus        — EventBus 广播（含 StreamOutput 预留）
├── context    — 连接上下文
├── exec       — 命令执行引擎
│   ├── process — 进程管理（进程组信号、优雅终止）
│   └── pty     — PTY 分配
├── ipc_handler — IPC 请求分发（cwd 继承、version 兼容检查）
├── lifecycle  — PID 文件、信号、socket 清理（ARSHY_TEST_NO_LIFECYCLE 绕过）
├── parser     — 解析引擎
│   ├── toml_def — TOML schema v1.0 格式定义（schema_version、deprecated、replaced_by）
│   ├── toml     — 无状态逐行匹配、stderr severity 修正
│   ├── rhai     — 有状态脚本引擎（Patterns + Script 双模式）
│   ├── crash    — 5 语言通用崩溃检测（Go/Python/Rust/Node/Shell）
│   ├── redos    — ReDoS 安全校验（嵌套量词、重叠交替）
│   ├── registry  — Parser 注册表 + 链式命令分段检测 + diff 审计
│   ├── detect   — 工具检测、版本探测、semver 缓存
│   ├── loader   — 文件系统热重载（notify v7）
│   └── json     — JSON 输出自动检测
├── security   — 命令过滤（Result 传播）、沙箱路径、审计日志（优雅降级）
├── store      — SQLite WAL、schema 迁移（v1→v2→v3）、索引优化
└── telemetry  — 原子计数器（tasks/events/connections）、stats/health 暴露
```

## 并发模型

- **单线程 tokio runtime**（proxy）：无需跨线程同步
- **多线程 tokio runtime**（daemon）：每个连接一个 spawn，接受循环受 `Semaphore(64)` 限制
- **事件总线**：`tokio::sync::broadcast`，多订阅者无阻塞
- **Parser 热重载**：`RwLock<ParserRegistry>`，读锁无阻塞，写锁短暂持有并输出 diff

## 存储

SQLite WAL 模式，表结构：

| 表 | 用途 | 索引 |
|----|------|------|
| `tasks` | 任务元数据 | `started_at`, `status` |
| `events` | 结构化事件 | `(task_id, seq)`, `(task_id, type, severity)` |
| `tool_versions` | 工具版本缓存 | — |
| `schema_version` | 迁移版本追踪 | — |

WAL 模式支持并发读写，checkpoint 自动清理。
