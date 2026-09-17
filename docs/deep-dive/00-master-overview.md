# arshy 宏观架构与核心业务全景图 (全量主文档)

> 本文档将原 `temporary.md` 的全量内容完整收录至 `docs/deep-dive/` 专栏中。包含 6 组核心架构/时序/状态机 Mermaid 流程图、图文解读以及符合 VS Code 标准的工作区相对源码跳转链接。

---

## 一、 系统宏观架构与组件拓扑 (Two-Binary Architecture)

```mermaid

graph TD
    subgraph ClientLayer["1. 接入层 (Clients)"]
        Agent["AI Agent / IDE<br/>(generic MCP client)"]
        CLIUser["人类开发者 CLI<br/>(arshy run / doctor / analyze)"]
    end

    subgraph ProxyLayer["2. 代理层: arshy (MCP Proxy)"]
        direction TB
        MCPStdio["MCP Stdio Bridge<br/>(tokio::select! 读写分离)"]
        DedupInject["Dedup Key 注入器<br/>(proxy_session_id + req_id)"]
        NotifBatcher["Notification Batcher<br/>(高频事件合并缓冲窗口)"]
        AutoStarter["Daemon Auto-Starter<br/>(按需拉起守护进程 + 退避重试)"]

        MCPStdio --> DedupInject
        DedupInject --> AutoStarter
        AutoStarter --> MCPStdio
    end

    subgraph IPCChannel["3. 进程间通信 (IPC)"]
        UDS["configured arshyd socket (Unix Domain Socket / JSON Lines)"]
    end

    subgraph DaemonLayer["4. 守护进程: arshyd (Daemon)"]
        direction TB
        IPCHandler["IPC Handler (Reader/Writer Split)"]
        EventBus["EventBus (Tokio Broadcast)"]

        subgraph ExecEngine["执行引擎 (Executor)"]
            RateLimit["RateLimiter & CommandFilter (安全沙箱)"]
            DedupCache["Replay Dedup Cache (120s TTL + Single-Flight 锁)"]
            TCCFallback["macOS TCC 绕过 (/tmp/.arshy-cwd/<hash>)"]
            Decision["路径决策 (is_short_command_with_route)"]
            PTYMgr["PTY 进程管理器 (libc + tokio)"]
            FastWorker["PTY capture (Fast Path 纯文本直出)"]
        end

        subgraph PipelineEngine["6 层解析流水线 + 规整器"]
            L1["L1: JSON/JSONL 检测"]
            L2["L2: Stateful 状态机"]
            L3["L3: TOML 正则规则 (38 个工具)"]
            L4["L4: Universal Crash (Panic/Traceback)"]
            L5["L5: Heuristic 启发式关键字"]
            L6["L6: Raw Log 兜底"]
            PairMerger["GenericPairMerger (跨行合并)"]
            RustcMerger["RustcContextMerger (源码指示线吸收)"]
            DedupEngine["Deduplicator (连续重复行折叠)"]
        end

        subgraph ContextEngine["本地世界事实中介 (Context)"]
            SourceEnrich["Source Slicer (本地文件 ±3 行源码切片)"]
            GitCorrelator["Git Correlation (Diff / Commit 关联)"]
            RootCauseExtr["Root Cause Extractor (关键首错/Traceback提取)"]
        end

        subgraph StorageEngine["存储与生命周期 (Store)"]
            TaskStore["tasks.jsonl (任务索引/指标)"]
            EventStore["events/<task_id>.jsonl (事件流)"]
            RefTable["Reference Table (受限错误码参考表 ADR-0001)"]
            PruneWorker["Prune Worker (LRU/TTL 自动轮转)"]
            IdleWatchdog["Idle Watchdog (300s 活动计时精确退出)"]
        end
    end

    Agent -->|MCP Stdio JSON-RPC| MCPStdio
    CLIUser -->|CLI Command Direct| AutoStarter
    AutoStarter <-->|UDS Stream| UDS
    UDS <-->|UDS Stream| IPCHandler

    IPCHandler --> ExecEngine
    RateLimit --> DedupCache --> TCCFallback --> Decision
    Decision -->|Raw 快速路径| FastWorker
    Decision -->|Structured 路径| PTYMgr

    PTYMgr --> PipelineEngine
    L1 --> L2 --> L3 --> L4 --> L5 --> L6
    L6 --> PairMerger --> RustcMerger --> DedupEngine

    DedupEngine --> ContextEngine
    ContextEngine --> StorageEngine
    DedupEngine -.->|实时广播| EventBus
    EventBus -.->|事件流转| IPCHandler
    IPCHandler -.->|流式通知| NotifBatcher
    NotifBatcher -.->|合并输出| MCPStdio
```

### 📖 业务逻辑与架构解读 (图一)
1. **双二进制架构（Two-Binary）**：
   - `arshy`（Proxy/CLI）：负责接收来自 Agent（Claude Code/Cursor）的标准输入输出（Stdio）JSON-RPC，完成参数检验与 `dedup_key` 注入；如果后台守护进程未启动，Proxy 负责将其按需拉起（Auto-start）。
   - `arshyd`（Daemon）：真正的常驻后台服务。管理 PTY、调度执行、流式解析、落盘 JSONL 以及 300s 无任务时自动退出。
2. **人类通道 vs Agent 通道（ADR-0003）**：人类使用 `arshy run "cargo build"` 时，直接通过 UDS 连接 Daemon 并获得色彩丰富的 Pretty UI 终端输出；Agent 则通过标准 MCP 协议通信，互不干扰。
3. **高频事件合并（Notification Batching）**：构建和测试往往瞬间产生上万行日志，Proxy 内部通过 `tokio::select!` 维护了时间窗口，将高频通知合并打包发送，防止 Agent 端 MCP 连接被泛洪打崩。

### 🔗 源码与 ADR 证据链
- **Proxy 事件循环与 Stdio 桥接**：[src/proxy/mod.rs:L100](../../src/proxy/mod.rs#L100) (第 100-220 行)
- **守护进程自动按需拉起与退避重连**：[src/proxy/connection.rs](../../src/proxy/connection.rs)
- **Daemon IPC 读写分离处理器**：[src/daemon/ipc_handler/mod.rs:L70](../../src/daemon/ipc_handler/mod.rs#L70) (第 70-120 行)
- **人类 CLI 直连实现**：[src/cli/mod.rs](../../src/cli/mod.rs) 与 [src/cli/render.rs](../../src/cli/render.rs)
- **架构决策依据**：[ADR-0003 (AI-Native 定位与人类通道)](../decisions/0003-ai-native-positioning-and-platform.md)、[ADR-0007 (数据驱动与按需 Daemon)](../decisions/0007-data-driven-execution-and-on-demand-daemon.md)

---

## 二、 命令执行路径与决策流 (Execution Path Decision Flow)

```mermaid
flowchart TD
    Start(["接收 arshy_exec 请求 (command, cwd, mode, parse_hint)"]) --> DedupCheck{"120s 幂等缓存是否命中?<br/>(MCP Request ID)"}

    DedupCheck -- 命中缓存 --> ReturnCached["直接返回已缓存的 RunResult<br/>(避免重试导致 rm/push 等副作用重复执行)"]
    DedupCheck -- 未命中 --> SingleFlight["获取 Key 专属 Single-Flight 锁<br/>(合并微秒级并发相同调用)"]

    SingleFlight --> RateLimitSec{"安全沙箱与速率校验<br/>(RateLimiter / CommandFilter / Path Check)"}
    RateLimitSec -- 命中黑名单/超频 --> ErrSec["拒绝执行并写入 AuditLog"]

    RateLimitSec -- 校验通过 --> TCCCheck{"CWD 是否受 macOS TCC 保护?<br/>(如 ~/Documents, ~/Desktop)"}
    TCCCheck -- 受保护路径 --> CreateSymlink["在 /tmp/.arshy-cwd/<hash> 创建软链接<br/>注入环境变量 ARSHY_CWD"]
    TCCCheck -- 正常路径 --> CarrierClassify["载体识别 classify_carrier<br/>(Shell / Composite / Python / Script)"]
    CreateSymlink --> CarrierClassify

    CarrierClassify --> MarkAct["刷新 Store::last_activity 活跃时间戳<br/>(保证只读命令也能刷新 300s 空闲看门狗)"]
    MarkAct --> RouteDecision{"is_short_command_with_route 判定"}

    RouteDecision -- "只读检查命令且无管道/危险参数<br/>(ls, cat, echo, pwd, rg 等)" --> FastPath["Raw Fast Path (快速路径)"]
    RouteDecision -- "命中了 Parser / 指定了 parse_hint /<br/>长命令 (>80字符/>5词) / 含管道与重定向" --> StructPath["Structured Path (结构化路径)"]

    subgraph FastPathFlow["Raw Fast Path (跳过结构化开销)"]
        FastPath --> FastPermit["获取执行许可"]
        FastPermit --> FastExec["PTY 直接捕获 (跳过 parser/store/event bus)"]
        FastExec --> FastOutput["捕获完整 stdout/stderr 文本"]
        FastOutput --> FastRet["返回 raw_output (不写 events.jsonl)"]
    end

    subgraph StructPathFlow["Structured Path (深加工路径)"]
        StructPath --> StructPermit["获取 task_semaphore 信号量<br/>(限制 max_concurrent_tasks 并发)"]
        StructPermit --> TaskRecord["写入 tasks.jsonl (Status: Running)"]
        TaskRecord --> SpawnPTY["PTY 伪终端拉起子进程"]
        SpawnPTY --> StreamPipe["6 层解析流水线 + 规整合并 + 源码切片"]
        StreamPipe --> DiskLog["流式写入 events/<task_id>.jsonl"]
        DiskLog --> ModeCheck{"Mode 执行模式判断"}

        ModeCheck -- "mode='auto' 且耗时 <= 60s<br/>或 mode='sync'" --> WaitComplete["同步等待完成<br/>提取 Root Cause + Git Context"]
        ModeCheck -- "mode='auto' 且耗时 > 60s<br/>或 mode='async'" --> DegradeAsync["降级为异步返回 task_id<br/>后续通过 EventBus/Query 查询"]
    end

    FastRet --> CacheSave["存入 120s 幂等缓存"]
    WaitComplete --> CacheSave
    DegradeAsync --> CacheSave
    CacheSave --> End(["响应 Agent / CLI 客户端"])
```

### 📖 业务逻辑与决策解读 (图二)
1. **重放幂等与 Single-Flight 防击穿（ADR-0004）**：Agent 在网络抖动重试时，相同 MCP Request ID 会命中 120s 结果缓存；若多个并发请求同时到达，Single-Flight 弱引用锁会阻塞后续请求，只执行一次命令，彻底杜绝 `rm -rf` 或 `git push` 被执行两次的灾难。
2. **macOS TCC 权限适配**：macOS 对 `~/Documents`、`~/Desktop` 等目录有严格的沙盒弹窗保护。`prepare_cwd` 检测到受保护目录时，会使用 `DefaultHasher` 生成标识，在 `/tmp/.arshy-cwd/<hash>` 下创建软链接并在子进程中注入 `ARSHY_CWD`，既保证权限不报错，又让命令内部感知真实路径。
3. **Raw Fast Path vs Structured Path（ADR-0007）**：
   - **Fast 路径**：针对 `ls`, `pwd`, `cat`, `rg` 等纯只读检查命令，仍通过 Unix PTY 捕获合并后的 stdout/stderr，但跳过 parser、事件落盘和 EventBus 广播，直接返回 raw output。若带危险参数（如 `tail -f`, `sed -i`, `sort -o`）或重定向/管道，强制转入 Structured 路径。
   - **Structured 路径**：针对构建、测试或长耗时任务，受 `max_concurrent_tasks`（默认 4）信号量控制，拉起真实 PTY 捕获色彩与行缓冲输出。在 `auto` 模式下，如果 60s 内未完成，自动从阻塞同步降级为返回 `task_id` 异步流转。

### 🔗 源码与 ADR 证据链
- **120s 幂等缓存与 Single-Flight 互斥锁**：[src/daemon/exec/mod.rs:L59](../../src/daemon/exec/mod.rs#L59) (第 59-65 行及第 215-260 行)
- **macOS TCC 软链接转换逻辑**：[src/daemon/exec/cwd.rs](../../src/daemon/exec/cwd.rs)
- **载体识别分类器（Carrier）**：[src/daemon/exec/decision.rs:L106](../../src/daemon/exec/decision.rs#L106) (第 106-164 行)
- **is_short_command AST 分析与白名单**：[src/daemon/exec/decision.rs:L19](../../src/daemon/exec/decision.rs#L19) (第 19-104 行)
- **Fast 路径实现 (run_short)**：[src/daemon/exec/mod.rs:L398](../../src/daemon/exec/mod.rs#L398) (第 398-412 行)
- **Structured 路径与 60s 自动降级**：[src/daemon/exec/mod.rs:L414](../../src/daemon/exec/mod.rs#L414) (第 414-447 行)
- **架构决策依据**：[ADR-0004 (幂等重放与并发安全)](../decisions/0004-replay-idempotency-and-execution-safety.md)、[ADR-0007 (数据驱动执行路径)](../decisions/0007-data-driven-execution-and-on-demand-daemon.md)

---

## 三、 6 层解析流水线与规整架构 (The 6-Layer Parser Pipeline)

```mermaid
flowchart TD
    RawLine["PTY 伪终端输出原始行"] --> L1{"1. JSON / JSONL 检测<br/>(try_parse_line)"}

    L1 -- 是 JSON --> EvJSON["生成 structured json 事件"]
    L1 -- 否 --> L2{"2. Stateful 状态机<br/>(stateful::feed_line)"}

    L2 -- 状态机捕获 --> EvState["生成 diagnostic / test_result 事件"]
    L2 -- 未捕获 --> L3{"3. TOML 正则规则库<br/>(38 个内置生态工具)"}

    L3 -- 正则捕获组匹配 --> EvTOML["提取 file, line, col, code, severity"]
    L3 -- 未匹配 --> L4{"4. Universal Crash 捕获<br/>(Panic / SIGSEGV / Traceback)"}

    L4 -- 命中堆栈特征 --> EvCrash["提取 crash 诊断事件"]
    L4 -- 未命中 --> L5{"5. Heuristic 启发式<br/>(关键字 error: / failed: 等)"}

    L5 -- 命中关键字 --> EvHeur["提取 warning / error 诊断事件"]
    L5 -- 未命中 --> L6["6. Raw Fallback 回退<br/>(type: log, severity: info)"]

    EvJSON & EvState & EvTOML & EvCrash & EvHeur & EvL6 --> PairM["GenericPairMerger<br/>(合并跨两行的错误模式: 如文件名行 + 错误信息行)"]

    PairM --> RustcM["RustcContextMerger<br/>(吸收 rustc 风格的 '|'、'-->'、'^^^^' 等指示线至主事件上下文)"]

    RustcM --> DedupM["Deduplicator<br/>(折叠连续完全相同的日志行，附带 repeat_count)"]

    DedupM --> EnrichM["Context Enrichment<br/>(本地文件系统读取报错位置 ±3 行源码切片，关联 Git Diff)"]

    EnrichM --> OutStream["流式持久化到 events/<task_id>.jsonl 并广播至 EventBus"]
```

### 📖 业务逻辑与规整解读 (图三)
1. **严格分层漏斗设计（6-Layer Pipeline）**：
   - **Layer 1 (JSON)**：优先识别原生结构化日志，零正则损耗直接转为事件。
   - **Layer 2 (Stateful)**：处理跨行状态机（如 pytest 的测试收集汇总）。
   - **Layer 3 (TOML Regex)**：核心层，内置 38 个生态工具（cargo, tsc, go, pytest, docker 等），通过 TOML 声明捕获代码位置与错误码。所有正则均通过 ReDoS 静态扫描。
   - **Layer 4 (Crash)**：通用兜底，捕获 Rust Panic、Python Traceback、Go Panic、SIGSEGV 等严重崩溃。
   - **Layer 5 (Heuristic)**：关键词启发式过滤，识别 `error:`, `failed:` 等通用警告。
   - **Layer 6 (Raw Fallback)**：兜底为普通 info log。
2. **多行规整与上下文吸收（Merger & Dedup）**：
   - `RustcContextMerger`：将编译器打印的 `  |`、`  -->`、`  = note:`、`^^^^` 等辅助视觉行全部合并进主 Diagnostic 事件的 `context.after` 数组中，防止视觉噪声稀释 LLM 上下文。
   - `Deduplicator`：在构建日志泛洪时，将连续出现的重复日志行折叠，记录 `repeat_count`。

### 🔗 源码与 ADR 证据链
- **6 层解析器引擎主分发**：[src/daemon/parser/mod.rs:L265](../../src/daemon/parser/mod.rs#L265) (第 265-303 行)
- **Rustc 上下文行吸收合并器**：[src/daemon/parser/mod.rs:L345](../../src/daemon/parser/mod.rs#L345) (第 345-420 行)
- **双行模式合并器 (PairMerger)**：[src/daemon/parser/pair_merger.rs](../../src/daemon/parser/pair_merger.rs)
- **连续行重复折叠器 (Deduplicator)**：[src/daemon/parser/dedup.rs](../../src/daemon/parser/dedup.rs)
- **ReDoS 静态正则安全检查**：[src/daemon/parser/redos.rs](../../src/daemon/parser/redos.rs)
- **38 个内置生态工具 TOML 规则资产**：[parsers/builtin/](../../parsers/builtin/)
- **架构决策依据**：[ADR-0006 (注意力编译器与三层载体)](../decisions/0006-value-anchor.md)

---

## 四、 同步结构化执行端到端时序图 (End-to-End Execution Sequence)

```mermaid
sequenceDiagram
    autonumber
    participant Agent as AI Agent (MCP Client)
    participant Proxy as arshy (MCP Proxy)
    participant IPC as UDS (configured arshyd socket)
    participant Daemon as arshyd (Daemon Exec)
    participant PTY as PTY 子进程 (libc + tokio)
    participant Pipeline as 6-Layer Pipeline
    participant Store as JSONL Store
    participant Bus as EventBus

    Agent->>Proxy: tools/call (name="arshy_exec", command="cargo build")
    Proxy->>Proxy: 提取 MCP Request ID 注入为 dedup_key
    Proxy->>IPC: {"method": "task/run", "params": {..., "dedup_key": "k_123"}}

    IPC->>Daemon: 调度 run_inner()
    Daemon->>Daemon: 速率与沙箱检查 -> 路径决策为 Structured 路径
    Daemon->>Store: insert_task(task_id, status: Running)

    Daemon->>PTY: spawn_command("cargo build")
    Daemon->>Bus: 广播 TaskUpdate(status: "running")

    loop PTY 输出流处理
        PTY-->>Daemon: 原始输出行 ("error[E0425]: cannot find value `foo`...")
        Daemon->>Pipeline: parse_line(line)
        Pipeline->>Pipeline: TOML 匹配 Rust 规则 -> 提取 file:src/main.rs, line:12
        Pipeline->>Pipeline: RustcContextMerger 吸收后续管道符号行
        Pipeline->>Store: append_event(task_id, TaskEvent)
        Pipeline->>Bus: publish(TaskEvent)
        Bus-->>Proxy: Notification (task/event)
        Proxy-->>Agent: MCP Logging / Notifications (按窗口合并批处理)
    end

    PTY-->>Daemon: 进程退出 (exit_code: 101)
    Daemon->>Pipeline: on_complete() 刷新状态机并触发最终事件
    Daemon->>Daemon: enrich_events() 本地文件读取源码 ±3 行切片
    Daemon->>Daemon: select_primary_diagnostic() 选择代表诊断
    Daemon->>Store: update_task(status: Failed, exit_code: 101)

    Daemon-->>IPC: 返回 RunResult (含 error_count, primary_diagnostic, project_context)
    IPC-->>Proxy: JSON-RPC Response
    Proxy-->>Agent: 响应精炼的结构化事实 (LLM 立即获取精准修复位置)
```

### 📖 业务逻辑与数据流解读 (图四)
1. **真实 PTY 进程管理**：使用 Unix PTY 与 `libc`/Tokio 建立虚拟终端，保证如 `cargo` 等工具能够输出完整无死锁的行缓冲数据。如果输出超过上限，会自动执行输出截断保护，但继续 drain 管道防止子进程阻塞挂起。
2. **本地世界事实中介（Context Enrichment）**：当捕获到包含 `file:line` 的报错事件后，Daemon 并不只返回错误文本，而是会自动读取本地磁盘对应文件的 **±3 行源码切片**，并关联当前的 **Git 未提交变更与最近 Commit**。LLM 收到响应后无需二次发起 `cat` 或 `git status` 查询即可直接开始修复。

### 🔗 源码与 ADR 证据链
- **后台任务调度与流式处理核心**：[src/daemon/exec/background.rs:L57](../../src/daemon/exec/background.rs#L57) (第 57-150 行)
- **PTY 进程创建与管道排空**：[src/daemon/exec/pty.rs](../../src/daemon/exec/pty.rs)
- **Root Cause 提取与 Python Traceback 特例**：[src/daemon/exec/enrich.rs:L1](../../src/daemon/exec/enrich.rs#L1) (第 1-60 行)
- **源码 ±3 行切片读取**：[src/daemon/context/mod.rs](../../src/daemon/context/mod.rs)
- **Git 变更与关联分析**：[src/daemon/context/git_correlator.rs](../../src/daemon/context/git_correlator.rs)

---

## 五、 诊断回溯与受限错误码查询时序 (Query & Reference Sequence)

```mermaid
sequenceDiagram
    autonumber
    participant Agent as AI Agent (MCP Client)
    participant Proxy as arshy (Proxy)
    participant Daemon as arshyd (Daemon)
    participant Store as JSONL Store
    participant RefTable as ReferenceTable (TOML)

    Agent->>Proxy: tools/call (name="arshy_query", task_id="t_123", errors_only=true)
    Proxy->>Daemon: task/query(task_id, errors_only=true)

    Daemon->>Store: read_task(task_id) & read_events_filtered(task_id)
    Store-->>Daemon: 返回过滤后的诊断事件列表

    opt 事件或任务包含非显而易见退出码 (如 docker 137 / 125)
        Daemon->>RefTable: lookup_code("docker", 137)
        RefTable-->>Daemon: 返回含义与权威链接: {"meaning": "OOM Killed", "source": "docs.docker.com..."}
        Note over Daemon: 遵循 ADR-0001 哲学: 仅附加标准含义与文档链接，严禁编造 Fix 建议
    end

    Daemon-->>Proxy: QueryResult (events + source_context + reference)
    Proxy-->>Agent: 返回结构化诊断事实与参考定义
```

### 📖 业务逻辑与哲学解读 (图五)
1. **去 HintDb 化与受限参考表（ADR-0001）**：项目彻底废除了过去基于规则合成修复建议（Cause/Fix/Retry）的 `HintDb`。因为**提出修复方案是 LLM 的职责，Parser 不应越俎代庖**。
2. **按需查表，永不内联**：仅针对 Docker 125/126/127/137、Kubectl、AWS 等非显而易见的系统级退出码维护权威参考表（`reference/builtin/*.toml`）。该参考表**永不写入事件持久化流**，仅在 Agent 通过 `arshy_query` 显式查询时按需附带标准释义和官方文档链接。

### 🔗 源码与 ADR 证据链
- **受限错误码参考表实现**：[src/daemon/reference/mod.rs](../../src/daemon/reference/mod.rs)
- **内置 Docker/K8s/AWS 退出码数据**：[reference/builtin/docker.toml](../../reference/builtin/docker.toml)
- **Query 处理器与参考码附加逻辑**：[src/daemon/ipc_handler/mod.rs:L250](../../src/daemon/ipc_handler/mod.rs#L250) (第 250-310 行)
- **架构决策依据**：[ADR-0001 (受限错误码参考表与去 HintDb)](../decisions/0001-restricted-reference-tables.md)

---

## 六、 守护进程生命周期与按需拉起状态机 (Lifecycle & Watchdog)

```mermaid
stateDiagram-v2
    [*] --> Stopped: 初始状态 (无后台常驻进程)

    Stopped --> ColdStart: MCP 客户端接入 / CLI 运行<br/>(Proxy 自动拉起 arshyd)

    state ColdStart {
        [*] --> RecoverTasks: 扫描 tasks.jsonl 恢复任务索引
        RecoverTasks --> MarkFailed: 将遗留的 Running 任务置为 Failed 终态 (PTY 不可重连)
        MarkFailed --> Ready: 启动 UDS 监听与 Watchdog
    }

    Ready --> ActiveRunning: 接收到命令执行请求 (Fast / Structured)

    state ActiveRunning {
        [*] --> Execute: 执行命令
        Execute --> RefreshActivity: 触发 mark_activity() 更新 last_activity
        RefreshActivity --> [*]
    }

    ActiveRunning --> IdleWaiting: 所有任务完成，无并发请求

    state IdleWaiting {
        [*] --> SleepDeadline: 计算 300s 剩余时间并精确睡眠
        SleepDeadline --> WakeOnActivity: 收到新命令 (中断睡眠)
        WakeOnActivity --> [*]
        SleepDeadline --> TimeoutReached: 达到 300s 且无活跃任务
    }

    IdleWaiting --> ActiveRunning: 收到新请求
    IdleWaiting --> Shutdown: 300s 空闲超时触发
    Ready --> Shutdown: 收到 SIGTERM / SIGINT 信号

    state Shutdown {
        [*] --> KillRunning: kill_all() 终止未完成任务
        KillRunning --> FlushStore: 刷新 Dirty Store 缓冲区
        FlushStore --> RemoveSock: 清理 configured socket 文件
    }

    Shutdown --> [*]: 进程优雅退出 (零常驻资源占用)
```

### 📖 业务逻辑与常驻优化解读 (图六)
1. **冷启动崩溃恢复（ADR-0007）**：Daemon 冷启动加载 `tasks.jsonl` 时，若发现上次异常崩溃遗留的 `Running` 状态任务，会立即将其更新为 `Failed`。因为 PTY 句柄无法跨进程重连，若保持 `Running` 会导致看门狗永久无法判定空闲。
2. **事件驱动的 300s 精确退出（Zero-Polling）**：摒弃了固定周期的计时器轮询，通过原子时间戳 `last_activity` 计算精确剩余秒数并让 Tokio 进入单次睡眠。所有命令（包括 Fast Path 快速路径）都会刷新该时间戳。
3. **安全关机与持久化保护**：进程在超时退出或收到 SIGTERM 信号时，会通过 `kill_all()` 终止所有子任务，将内存中 Dirty 的 JSONL 缓冲区安全同步至磁盘，并清理 UDS Socket 文件，实现零常驻资源消耗（Zero Resident Resource）。

### 🔗 源码与 ADR 证据链
- **Daemon 主入口与 300s 精确看门狗循环**：[src/daemon/main.rs:L180](../../src/daemon/main.rs#L180) (第 180-290 行)
- **冷启动任务索引恢复与终态置位**：[src/daemon/store/tasks.rs](../../src/daemon/store/tasks.rs)
- **Store 活跃标记与安全 Flush 机制**：[src/daemon/store/mod.rs:L94](../../src/daemon/store/mod.rs#L94) (第 94-100 行)
- **架构决策依据**：[ADR-0002 (JSONL 存储维持与触发线)](../decisions/0002-jsonl-retention-and-scale-trigger.md)、[ADR-0004 (空闲退出修复)](../decisions/0004-replay-idempotency-and-execution-safety.md)、[ADR-0007 (事件驱动 Daemon 与冷启动恢复)](../decisions/0007-data-driven-execution-and-on-demand-daemon.md)
