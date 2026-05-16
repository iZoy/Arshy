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
                                                      │         │    │ Engine    │   │ (SQLite)    │
                                                      └────┬────┘    └───────────┘   └─────────────┘
                                                           │
                                                      ┌────▼────┐
                                                      │Process  │
                                                      │(sh -c)  │
                                                      └─────────┘
```

## 三个进程

### arshy（proxy / CLI）

入口：`src/main.rs`

两种运行模式：
- `--from-mcp`：MCP stdio proxy，连接 Claude Code / Cursor
- 子命令：CLI 直接调用（`arshy run`、`arshy list` 等）

两种模式最终都通过 Unix Domain Socket 与 daemon 通信。

### arshyd（daemon）

入口：`src/daemon/main.rs`

后台常驻进程，职责：
1. 监听 UDS 连接
2. 执行命令（Executor）
3. 解析输出（Parser Engine）
4. 持久化数据（Store）
5. 安全过滤（Security）

生命周期：
```
启动 → 写 PID → 检查 socket → 绑定 UDS → accept 循环
                                                    ↓ (ctrl-c 或 daemon/shutdown)
清理 socket → 删除 PID → 退出
```

### Agent（外部）

Claude Code、Cursor 等 MCP 客户端。通过 stdio 与 proxy 通信，感知不到 daemon 的存在。

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
                                      └─────────┘  │启动进程      │
                                                   │Parser管道    │
                                                   │事件写入Store │
                                                   │通知推送到Proxy│
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

第一级命中则跳过后续级别。详见 [Parser 管道](parser-pipeline.md)。

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
├── config     — 配置加载、环境变量扩展
├── error      — ArshyError 枚举
├── ipc        — JSON-RPC 类型、UDS 传输、错误码
└── mcp        — MCP 协议类型、工具/资源/指令定义

arshy (crate: bin, proxy)
├── cli        — clap 子命令分发
└── proxy      — MCP stdio 代理

arshyd (crate: bin, daemon)
├── bus        — EventBus 事件广播
├── context    — 连接上下文
├── exec       — 命令执行引擎
│   ├── process — 进程管理
│   └── pty     — PTY 分配（预留）
├── ipc_handler — IPC 请求处理
├── lifecycle  — PID 文件、信号、socket 清理
├── parser     — 解析引擎
│   ├── toml_def — TOML 格式定义
│   ├── toml     — 无状态逐行匹配
│   ├── rhai     — 有状态脚本引擎
│   ├── crash    — 通用崩溃检测
│   ├── registry — Parser 注册表
│   ├── detect   — 工具检测
│   └── loader   — 文件系统热重载
├── security   — 命令过滤、审计日志
└── store      — SQLite 持久化
```

## 并发模型

- **单线程 tokio runtime**（proxy）：无需跨线程同步
- **多线程 tokio runtime**（daemon）：每个连接一个 spawn，Executor 内部用 `Arc<Mutex<...>>` 保护共享状态
- **事件总线**：`tokio::sync::broadcast`，多订阅者无阻塞
- **Parser 热重载**：`RwLock<ParserRegistry>`，读锁（解析）无阻塞，写锁（重载）短暂持锁

## 存储

SQLite WAL 模式，四张表：
- `tasks`：任务元数据
- `events`：结构化事件
- `tool_versions`：工具版本缓存
- `parser_registry`：已加载 parser 记录

WAL 模式支持并发读写，checkpoint 自动清理。
