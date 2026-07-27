# 设计原则

## 核心约束

Arshy 围绕六个约束设计：

### 1. 零开销短路径

简单命令（`ls`、`echo hello`、`grep -rn pattern src/`、`git diff`）必须以原生 shell 的速度执行。

实现：`is_short_command()` 综合判断 — 40+ 个只读检查工具（echo/cat/ls/grep/find/git 等）绕过字数/长度限制，build/test 命令强制走结构化路径，链式操作（`&&`/`||`/`;`）拒绝短路径。短命令跳过 Store、Parser、EventBus，直接返回 stdout。

### 2. 结构化事件，非文本流

AI Agent 需要机器可解析的输出，而非人类可读的日志。

设计决策：20 个内置 parser（31 个 pattern）覆盖主流工具。输出经 4 级管道（Stateful → Line → Crash → Raw）产生 `TaskEvent`（type/severity/code/file/line）。Agent 通过 `arshy_query` 查询类型化事件，而非 regex-parse 原始文本。

### 3. 持久会话，非一次性进程

命令执行的结果必须可查、可追、可统计。

设计决策：所有任务和事件存入 SQLite WAL。Agent 可随时查询历史任务、过滤事件、查看位置。数据库支持自动清理和 schema 迁移（v1→v2→v3）。

### 4. 安全边界

Agent 可执行任意命令，但系统必须有多层安全兜底。

设计决策：四层安全 — 命令黑名单正则（`Result` 传播，不 panic）、可选白名单模式、沙箱路径限制、审计日志（优雅降级，写失败不崩溃）。Socket 权限 0600（owner-only）。沙箱模式校验（不支持的模式启动时明确拒绝）。

### 5. 可扩展解析

新工具的输出格式不断出现，解析能力必须可扩展且可维护。

设计决策：三级 parser 架构 — TOML 声明式（schema v1.0，含 deprecated/replaced_by/since_version，支持无状态与有状态模式）、Crash/Raw 兜底。Parser 热重载 + diff 审计。`ARSHY_BLESS=1` 自动生成 test fixture。ReDoS 静态校验拒绝危险正则。

### 6. MCP 原生，自描述

Arshy 是为 AI Agent 设计的，不是为人类设计的 CLI 包装器。

设计决策：MCP initialize 时宣告 `experimental.preferredShell` + 权威 `instructions`。Agent 自然获知 arshy 是主 shell，无需配置 CLAUDE.md。所有 MCP 错误响应统一携带 `data.retryable` 字段供 agent 决策。

## 架构决策

| 决策 | 理由 |
|------|------|
| Unix Domain Socket（0600） | 本地安全性、文件权限控制、无端口冲突 |
| JSON Lines 而非 WebSocket | 简单、无额外依赖、tokio 原生支持 |
| SQLite WAL 而非文件日志 | 查询能力、并发读写、自动清理、schema 迁移 |
| 后台 daemon + 空闲退出 | 启动延迟 <5ms、状态持久化、5min 无活动自动退出 |
| 进程组信号 | 终止任务时同时 kill 子进程，避免孤儿进程 |
| 单 tokio runtime（proxy）/ 多线程（daemon） | 简化内存模型、连接隔离 |
| Semaphore(64) 连接限制 | 防止资源耗尽 |
| 健康检查指数退避 1s→60s | 防止雷群效应 |

## 不做的事

- 不代理网络请求（只管 shell）
- 不管理虚拟环境或容器（sandbox_mode 预留，当前仅 `none`）
- 不提供 UI（纯协议层）
- 不缓存命令输出（只存事件元数据）
- 不替换 shell（使用系统 `sh -c`）
