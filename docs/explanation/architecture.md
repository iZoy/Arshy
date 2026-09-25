# 系统架构：两个进程，一条执行链

arshy 的架构围绕一个核心判断展开：**命令执行的"价值"与"开销"取决于命令本身，而不取决于调用方式**。短命令（`ls`、`git status`）的价值是捕获文本，任何包装都是浪费；长命令（`cargo build`、`npm test`）的价值在于错误的结构化信息，整段捕获文本反而难以消化。

因此 arshy 不做一个"包一层壳的 bash"，而是一个由两个进程组成的执行层：一个轻量代理负责与 AI 客户端打交道，一个常驻守护进程负责真正执行与结构化。本文解释这两个进程为什么分开、如何通信、各自如何存活，以及命令如何在其中走不同的执行路径。

## 两个二进制，一种职责划分

Cargo.toml 声明了两个二进制目标（`[[bin]]`）：

| 二进制 | 角色 | 主要模块 |
|---|---|---|
| `arshy` | 代理 / CLI | `src/proxy/`（MCP stdio 代理：`mod.rs` + `handlers.rs`/`connection.rs`/`protocol.rs`）、`src/cli/mod.rs`（CLI 命令） |
| `arshyd` | 守护进程 | `src/daemon/main.rs`（启动与接受循环）、`src/daemon/ipc_handler/`（请求分发）、`src/daemon/exec/`（执行引擎） |

`arshy` 是"门面"：它作为通用 MCP server（`arshy mcp serve`）通过 stdio 与任意 MCP 客户端对话，也作为 CLI 接受 `arshy run "..."` 这类命令。它自己**不执行任何 shell 命令**，只把请求翻译成 IPC 协议转发给 `arshyd`。

`arshyd` 是"引擎"：它拥有命令子进程、解析管线、JSONL 存储、事件总线与安全策略。它是唯一真正 `spawn` 命令的进程。

把两者分开的收益是**生命周期解耦**：代理进程的生命周期与客户端绑定（客户端退出它就退出），守护进程的生命周期与"有没有活干"绑定。这样同一个守护进程可以被多个 MCP 客户端和 CLI 调用，任务历史也不会因为代理退出而丢失。

## 通信拓扑：两段协议，一个消息模型

```mermaid
flowchart LR
    A[AI 客户端<br/>Claude Code / Cursor / Codex] -->|"MCP JSON-RPC 2.0<br/>stdio 逐行"| P[arshy 代理]
    P -->|"IPC JSON-RPC 2.0<br/>Unix Domain Socket<br/>JSON Lines"| D[arshyd 守护进程]
    D -->|"sh -c 子进程<br/>stdout/stderr 管道逐行"| C[工具进程<br/>cargo / python / make ...]
    P -.->|"通知：task/update<br/>task/complete / diagnostic"| A
```

三段链路各有原因：

- **客户端 ↔ 代理（stdio）**：MCP 的 stdio transport 把代理的进程生命周期绑定到客户端进程上，客户端怎么启动、怎么退出，代理就怎么跟着走。代理不需要自己做服务发现或进程管理。
- **代理 ↔ 守护进程（UDS + JSON Lines）**：Unix Domain Socket 让本机任意客户端都能连上同一个守护进程；JSON Lines 逐行帧与 stdio 逐行帧形态一致，代理转发几乎不需要缓冲重组。Socket 默认在 `${XDG_DATA_HOME}/arshy/arshyd.sock`，权限 `0o600`（仅属主可访问，见 security-model）。
- **守护进程 ↔ 命令（`sh -c` 管道）**：命令以非交互式 `sh -c "<cmd>"` 子进程启动（`src/daemon/exec/pty.rs`），stdin 接 `/dev/null`，stdout/stderr 是彼此独立的管道，由异步 reader 逐行送入 channel。它不分配真正的伪终端，不提供交互输入，也不保证程序检测到 TTY。`TERM=dumb` 是单独设置的环境变量，不能让管道变成终端。模块名 `pty` 是历史命名。交互式 PTY 未实现。

守护进程首次执行命令时，会用用户登录 shell 探测并缓存 PATH（`user_shell_path()`），供 `sh -c` 子进程使用。这只补充 PATH；命令本身仍不是登录 shell，其他 profile 设置、交互式功能和终端行为都不因此继承。stdin 为 EOF，且 `TERM=dumb`，所以调用者应使用非交互命令。

## 代理做了什么

`src/proxy/mod.rs` 的核心是一个 `tokio::select!` 三路循环，同时监听 stdin（客户端请求）、守护进程通知通道（事件推送）、以及 SIGTERM（父进程退出信号）。代理本身不做任何业务判断，但有几个值得理解的设计：

- **通知批处理**：守护进程会推送高频事件（`task/update`、`diagnostic`）。代理把它们放进待发队列，默认每 100ms 或攒满 50 条（`notifications.batch_interval_ms` / `max_batch_events`）合并成**一条** `notifications/message`，payload 是 JSON 数组。高频事件按"窗口"合并而不是逐条转发，直接降低了客户端上下文中的噪音。工具调用完成后还会立即 `drain_pending()` 把剩余通知冲刷出去，保证结果与事件顺序一致。
- **协议版本协商**：MCP 握手时回显客户端请求的版本（支持 2024-11-05 到 2025-11-25 的四个稳定版本），未知版本回退到最早稳定版。注释里记录了一个真实教训：用过期硬编码版本号曾导致真实客户端握手失败。
- **取消传播**：客户端发 `notifications/cancelled` 时，代理用 `request_tasks` 表把 MCP request id 映射回 daemon 的 task_id，并发送 `task/kill`。MCP 的取消语义在代理层被翻译成守护进程的杀任务语义。
- **工具面单一职责**：MCP 暴露 `arshy_exec`（只执行）、`arshy_query`（只查结构化诊断）与 `arshy_task`（低频 cancel/list/raw）。三个简单 schema 比一个包含大量条件字段的 action 联合体更容易让模型正确选择。
- **中间件链**：`Middleware` trait 预留了请求/响应/通知三层钩子（注释提到 AuditMiddleware、RateLimitMiddleware、AuthMiddleware 为将来用途），当前为空链。它是一处"可扩展 > 硬编码"的架构接缝，而不是已实现的机制。

## 守护进程生命周期：按需自启、空闲退出、请求驱动自愈

守护进程不需要用户手动管理，这是整个生命周期设计的出发点。四个机制配合实现"用完即走、需要即在"：

### 1. 按需自启（on-demand auto-start）

`connect_or_start()`（`src/proxy/connection.rs`）先尝试连 socket：连得上直接用；连不上且 `daemon.auto_start`（默认开）时，清理疑似 stale 的 socket 文件，然后 `start_daemon()`：

- **spawn-lock 防重复**：`/tmp/arshyd.spawn-lock` 用 `create_new` 原子创建，写入 PID。若锁已被持有，说明另一个代理正在拉起守护进程，当前调用直接放弃并轮询等待 socket 出现。
- **崩溃熔断**：若 2 分钟内累计 5 次启动/连接失败（`MAX_CRASHES=5`、`CRASH_WINDOW_SECS=120`），自动启动被抑制，返回可读的 `DaemonUnreachable` 错误。这是防止"守护进程 crash-loop、代理疯狂重启它"的自我保命机制。
- **脱离会话**：`setsid()` 让守护进程脱离代理的进程组和控制终端——即使发起者（如 agent 的 shell）先退出，守护进程仍能活到任务结束或空闲超时。
- **二进制解析**：优先找"当前可执行文件同目录下的 arshyd"并用 `canonicalize` 解析真实路径。这条注释解释了为什么：当 arshy 通过 `~/.arshy/bin/bash` 符号链接被调用时，`current_exe()` 返回的是符号链接路径，直接取父目录会找不到 arshyd。
- 启动后以 40 次 × 250ms 轮询等待 socket 就绪。

### 2. 启动重试与存活校验

代理启动时 `connect_with_retry` 最多尝试 5 次（间隔 500ms），每次成功后发一个 `session/cd` 请求验证连接确实可用（"auto-cd validation"）——连接成功不等于协议可用。

### 3. 请求驱动的自愈（self-heal on demand）

如果运行中连接断开（`is_connection_error` 识别 connection closed / timed out / broken pipe / refused / missing socket），代理在 `ensure_daemon_up` 中先冲刷陈旧通知、再以 500/1000/2000ms 退避重连，必要时重新拉起守护进程，然后**把当前请求重放一次**。对 agent 来说，一次连接抖动表现为"稍慢的请求"，而不是一次失败。`tools/call`、`resources/list`、`resources/read` 都走这个恢复路径。

### 4. 空闲退出（idle-exit）

`src/daemon/main.rs` 使用事件驱动的空闲截止时间：默认空闲阈值 300 秒（5 分钟，`idle_timeout_secs`，0 表示禁用）。每次命令开始或结构化任务完成都会刷新 deadline；任务运行期间 deadline 暂停。这里没有每秒或每分钟轮询，Store 的后台 flush 也只由脏数据通知唤醒。这样 IDE 关闭、代理退出后，daemon 会自动退出，而空闲期间不会为了检查“是否空闲”反复唤醒 CPU。

### 5. 优雅关闭

关闭路径分三阶段（`src/daemon/main.rs`）：

1. 广播 `daemon/shutdown` 通知（30s 宽限期），立即删除 socket 拒绝新连接；
2. 先给运行中任务 5 秒自然完成；超时后 `executor.kill_all()`，硬截止 30 秒后强制退出；
3. flush store、删 PID 文件、删 socket。

代理收到 SIGTERM 时也会尽力向守护进程发一次 `daemon/shutdown`（best-effort），让"客户端退出"这个最常见的场景把守护进程也带走。

## 两种执行路径：原始快速路径 vs 结构化路径

`src/daemon/exec/mod.rs` 把执行分成三种模式：`sync`（等到底）、`async`（立即返回 task_id）、`auto`（默认，智能选择）。**auto 模式是 arshy 对"什么时候值得结构化"的答案**：不值得结构化的命令一分钱都不花，值得结构化的命令一次调用给全。

### 路径判定（`is_short_command`）

| 规则 | 选择 |
|---|---|
| 生命周期语法 | `&&`、`||`、`;`、后台运行或重定向进入结构化路径；普通只读管道可走快速路径 |
| 长跑信号 | `--watch`、`-f`、`serve/server`、`daemon`、`start`、`dev`、`preview` 进入结构化路径 |
| parser 资产 | registry 命中任意内置或用户 TOML parser，进入结构化路径 |
| 明确例外 | echo/cat/ls/rg 等检查工具，以及 git status/log/diff 等只读操作，返回原始文本 |
| 无 parser 兜底 | 超过 80 字符或 5 个词进入结构化路径，否则快速返回 |

关键点是不存在 build/test Rust 前缀表。路径式调用与多词命令由 parser registry 的 `detect`/`detect_full` 识别；新增 parser TOML 后，匹配命令会自动取得结构化执行资格。Rust 只保留跨生态稳定的 shell 生命周期规则与只读例外。

### 短路径：零开销直通

`run_short()` 直接 spawn、等待，并将 stdout/stderr 两个 reader 收到的行按 channel 到达顺序拼成 `raw_output`；每行来源标签被忽略，因此跨流相对顺序不保证。它**跳过** store 插入、parser session 和 EventBus——这三个都是为结构化服务的。但两条例外很关键：**安全过滤和审计日志不跳过**（见 security-model 的"安全默认非可选"）。超时处理也存在：超时则 `force_kill` 并返回 timeout 状态。

### 结构化路径：完整流程

parser-backed 或生命周期敏感命令走完整链路：

1. 工具检测：`parser.detect(command)` 从 38 个内置 TOML 解析器 + 用户解析器中选出工具（`parse_hint` 可强制指定）；
2. 创建 task（uuid task_id，写入 store，状态 running），spawn 后台执行任务；
3. 输出逐行进入 6 层解析管线（见 parser-pipeline），事件去重、合并、存入 JSONL、发布到 EventBus；
4. **auto 的同步耐心窗口是 60 秒**：命令在 60 秒内结束，则同步返回完整结构化结果；超过 60 秒，降级为 async——返回 `task_id` 和 `running` 状态，任务继续在守护进程里跑，agent 可以 `arshy_query`、`arshy kill` 或订阅；
5. 完成后从完整错误事件集合选择 `primary_diagnostic`（traceback 场景取最后一个诊断错误）；不读取源码文件，也不关联 Git 变更；
6. 响应里内联事件按结果裁剪：失败最多 20 条 error 事件，成功最多 5 条 warning/info 事件，超出部分用 `events_truncated` + `events_hint` 告诉 agent "去 `arshy_query` 取全量"。

显式 `sync` 模式没有 60 秒耐心窗口（调用者选择了阻塞）；显式 `async` 立即返回。任何 `parse_hint` 都会强制走结构化路径——调用方声明了期望格式，就按结构化兑现。

### 为什么短命令不"顺便"结构化？

因为结构化有真实成本：store 写入、解析 session、事件流、去重合并，以及最重要的——**输出给 agent 的形态变化**。`echo hi` 的价值就是那两个字；把它变成一条 JSON 事件反而制造噪音。短/长分界本质上是对"解析收益 > 解析成本"的显式建模，而不是实现妥协。

## 进程树与信号语义

```mermaid
flowchart TD
    Client[AI 客户端] -->|stdio| Proxy[arshy 代理]
    Proxy -->|UDS| Daemon[arshyd 守护进程<br/>setsid 独立会话]
    Daemon -->|sh -c| Cmd1[sh -c 子进程<br/>命令进程树根]
    Cmd1 --> Tool1[cargo / python ...]
    Cmd1 --> Tool2[子进程链]
```

- 每个命令是 `sh -c` 的一个子进程，工具派生的孙进程构成命令进程树；
- `kill_on_drop(true)` 保证守护进程死亡时子进程不被遗弃；
- 杀任务用信号升级：`SIGINT →（等 3s）→ SIGTERM →（等 2s）→ SIGKILL`，每步同时向进程组（负 PID）与直接 PID 发送信号，以覆盖管道与 fork 出的子进程链；
- 短路径与长路径共用同一套 spawn/读取/杀灭机制，只是跳过解析层。

## 一个特殊适配：macOS TCC

`src/daemon/exec/cwd.rs` 会识别位于 `~/Documents`、`~/Desktop`、`~/Downloads` 下的 cwd，并尝试在 `/tmp/.arshy-cwd-<uid>/` 创建一个私有符号链接作为子进程 cwd，同时注入 `ARSHY_CWD` 保存请求的真实路径。符号链接只是路径别名，不会改变目标文件权限；此方法是否改变 macOS TCC 对该进程的授权行为尚未验证，不能视为权限绕过或安全边界。

## 关键取舍

| 取舍 | 选择 | 原因 |
|---|---|---|
| 一个进程还是两个 | 两个 | 生命周期解耦：代理随客户端生死，守护进程按需独立存活 |
| 代理做业务吗 | 不做 | 代理越薄，越不容易与客户端协议绑定；业务全在守护进程，单一职责 |
| 检查命令结构化吗 | 通常不 | 解析收益小于成本；原始文本对只读工具最有用 |
| 结构化命令同步还是异步 | 先同步 60s，再降级异步 | 大多数构建在 60s 内结束，一次往返拿到结果最省事；超长任务不阻塞调用方 |
| 守护进程何时退出 | 空闲 5 分钟 | 事件驱动 deadline，无周期 idle 轮询 |
| 连接抖动怎么办 | 自动重连 + 请求重放 | 对 agent 表现为延迟而非失败 |
