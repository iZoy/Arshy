# arshy 核心架构与业务逻辑深度剖析 (Deep Dive)

本文档库专为开发者深入理解 `arshy` 的业务逻辑实现、架构分层与设计决策而建。基于项目当前源码（`src/`）与全部 8 份 ADR 架构决策记录（`docs/decisions/`）整理。

---

## 🗂️ 专题文档导航

```
docs/deep-dive/
├── README.md                          # [当前文档] 专栏介绍与 ADR 对照矩阵
├── 00-master-overview.md              # [全景主文档] 包含全部 6 组架构/时序/状态机图与完整解读
├── 01-proxy-mcp.md                    # 代理层与 MCP 协议栈 (Stdio/Session Nonce/Resources)
├── 02-execution-decision.md           # 执行路径决策与安全沙箱 (Shell AST/TCC 绕过/Fast vs Struct)
├── 03-parser-pipeline.md              # 6 层流式解析器流水线 (TOML 规则/ReDoS/多行规整)
├── 04-pty-process-context.md          # PTY 进程管理与本地世界中介 (管道排空/源码切片/Git 关联)
├── 05-store-reference-lifecycle.md    # 存储引擎、受限参考表与生命周期 (JSONL 句柄池/300s 看门狗)
└── 06-source-audit-workbook.md        # 源码事实核验、Rust 实验与发布 Go/No-Go 工作簿
```

阅读顺序建议先看专题，再以 [06-source-audit-workbook.md](06-source-audit-workbook.md)
逐章回到源码、测试和发布命令核验。专题文档中的路径、图示和结论都不是最终证据；工作簿记录
当前源码基线、已确认事实、待核验问题和发布风险。

---

## 一、 系统全景宏观拓扑图

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

---

## 二、 ADR 架构决策与源码对照矩阵

| ADR 编号 | 核心决策 | 解决的痛点与依据 (Rationale) | 对应的源码实现位置 |
| :--- | :--- | :--- | :--- |
| **[ADR-0001](../decisions/0001-restricted-reference-tables.md)** | **受限错误码参考表**（彻底废除 HintDb） | 修复建议（Fix/Cause）是 LLM 的职责，Parser 不应越俎代庖；仅将 Docker 137/AWS 等非直观系统退出码含义按需提供。 | [src/daemon/reference/](../../src/daemon/reference/)<br/>[reference/builtin/](../../reference/builtin/) |
| **[ADR-0002](../decisions/0002-jsonl-retention-and-scale-trigger.md)** | **维持 JSONL 存储 + 规模触发线** | 文本追加写、零外部 DB 依赖、高可调试性。设定硬性触发线：任务数 >10k 或延迟 >200ms 才引入派生索引。 | [src/daemon/store/mod.rs](../../src/daemon/store/mod.rs)<br/>[src/daemon/store/prune.rs](../../src/daemon/store/prune.rs) |
| **[ADR-0003](../decisions/0003-ai-native-positioning-and-platform.md)** | **AI-Native 定位与保留人类通道** | 默认作为 Agent 执行后端，同时保留人类 CLI（`arshy run`）与 Pretty 输出。专注于 Unix-like 环境。 | [src/main.rs](../../src/main.rs)<br/>[src/cli/mod.rs](../../src/cli/mod.rs) |
| **[ADR-0004](../decisions/0004-replay-idempotency-and-execution-safety.md)** | **请求重放幂等 + 并发上限 + 空闲修复** | 解决网络重试导致副作用命令双重执行；落地 `max_concurrent_tasks` 信号量；原子记录短命令活跃以修复看门狗。 | [src/daemon/exec/mod.rs](../../src/daemon/exec/mod.rs#L59)<br/>[src/proxy/handlers.rs](../../src/proxy/handlers.rs) |
| **[ADR-0006](../decisions/0006-value-anchor.md)** | **三大价值锚点与三层载体策略** | 注意力编译器 + 本地事实中介（Git/源码切片）+ 社区规则资产；区分轻量检查、重型工具与脚本三层载体。 | [src/daemon/context/](../../src/daemon/context/)<br/>[parsers/builtin/](../../parsers/builtin/) |
| **[ADR-0007](../decisions/0007-data-driven-execution-and-on-demand-daemon.md)** | **数据驱动执行 + 单一职责 MCP** | 区分 Raw Fast Path 与 Structured Path；MCP 明确收敛为 `arshy_exec` / `arshy_query` / `arshy_task` 3 个工具。 | [src/daemon/exec/decision.rs](../../src/daemon/exec/decision.rs)<br/>[src/mcp/](../../src/mcp/) |
| **[ADR-0008](../decisions/0008-prompt-first-agent-onboarding.md)** | **Prompt-First Agent 接入引导** | 避免为每个客户端维护脆弱的 Rust Adapter，通过 `arshy mcp config --format prompt` 输出标准引导让 Agent 自主适配。 | [src/cli/mod.rs](../../src/cli/mod.rs) |
