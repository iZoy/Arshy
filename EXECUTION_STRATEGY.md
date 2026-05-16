# Arshy 执行策略计划

> **基准日期**：2026-05-15
> **核心理念**：AI Agent 唯一的 Shell。透明、结构化、可控。零兼容包袱。

---

## 全局现状

| 指标 | 值 |
|------|-----|
| 已完成 Stage | A, B, C, D, E, F, G, H, I, J, K, L, M, N*, O, P, S |
| 总测试数 | **277** (31 lib + 13 proxy + 233 daemon) |
| Clippy | 0 warnings |
| 编译器警告 | 0 |
| 依赖 | `notify` v7 ✅, `rhai` v1.24 ✅ (已解锁) |
| 状态 | **Phase 1-4 全部完成** |

---

## 执行策略：四阶段推进

```
Phase 1 (P0)      Phase 2 (P1)       Phase 3 (P2)       Phase 4 (P3)
Agent 可接入      生产可靠性          长期运行            生态完善
                                                         
J→P→S  ───────→  K→L  ───────→  M  ───────→  N→O
│                 │                 │                 │
│ 做完 Agent      │ 做完可部署      │ 做完可运维      │ 做完可扩展
│ 才敢连上来      │ 到生产环境      │ 长期不泄漏      │
```

每个 Phase 内的 Stage 按字母序串行，Phase 之间产生可交付增量。

---

## Phase 1: Agent 可接入（P0，核心价值交付）

> 目标：做完这三步，Agent 可以安全地通过 arshy 执行所有 shell 命令，短命令零开销，长命令结构化。
> 不做 = 不敢让 Agent 连上来。

### Stage J — 通知实时性

**当前状态**：proxy 主循环在 `read_line(stdin)` 阻塞，通知只在读完一行后才 drain。EventBus → daemon channel → proxy 链路已通，但 proxy 侧无法在等待 stdin 时同时转发通知。

**关键文件**：
- `src/proxy/mod.rs:40-97` — proxy_main loop
- `src/daemon/ipc_handler.rs:58-80` — daemon reader/writer 双 task
- `src/daemon/bus/mod.rs` — EventBus 发布/订阅

**要做的事**：

| Step | 内容 | 探索深度 |
|------|------|----------|
| J1 | Proxy 双向 select：主循环改为 `tokio::select!` 同时监听 stdin 和 daemon notification channel | 充分 — 目标明确，`DaemonConnection` 已有 `drain_notifications()`，只需把 polling 改为 select |
| J2 | Notification forwarder task：独立 tokio task 从 daemon channel 读取 → 写入 stdout | 🧭 探索性 — 需确认是否需要独立 task，还是 select 方案已足够。当前 `drain_and_forward_notifications` 是 polling 模式，需实测性能 |
| J3 | 通知缓冲：高频事件合并 batch，config `batch_interval_ms` 已有字段 | 充分 — config schema 已预留 `batch_interval_ms` 和 `max_batch_events`，直接接线 |
| J4 | 通知测试：验证 async 模式下 Agent 发 run 后即时收到 update/complete | 🧭 探索性 — 需设计测试方案（时序验证），当前测试 infrastructure 已有 UDS pair，可复用 |

**产品对齐检查**：
- ✅ 不影响短命令零开销（通知只在 async 模式触发）
- ✅ 不改 MCP 协议，只改内部 I/O 模型
- ✅ 无兼容包袱

---

### Stage P — Agent 无缝接入

**当前状态**：executor 中 `mode == "auto"` 被简单视为 sync（`src/daemon/exec/mod.rs:145`），没有智能判断。短命令仍走完整 Store + Parser 路径。MCP instructions 仍描述 5 工具模型。

**关键文件**：
- `src/daemon/exec/mod.rs:94-231` — Executor::run()
- `src/ipc/mod.rs:138-148` — RunTaskParams（尚无 parse_hint 字段）
- `src/mcp/instructions.rs` — tool_definitions() + default_instructions()
- `src/proxy/mod.rs:294-303` — mcp_tool_to_ipc_method()

**要做的事**：

| Step | 内容 | 探索深度 |
|------|------|----------|
| P1 | mode:auto 智能推断：实现 `is_short_command()` 判定函数（见 ROADMAP 伪代码），短命令走 sync 路径 | 充分 — 判定规则已在 ROADMAP 中明确给出 |
| P2 | 短命令零开销路径：`Executor::run()` 中 auto + 短命令判定 → 跳过 `insert_task`、parser session、直接 spawn → wait → return stdout 原文 | 充分 — 改动集中在 `Executor::run()`，需注意审计日志仍需记录（安全要求） |
| P3 | 输出格式自动切换：短命令 MCP response 返回 `{"text": "..."}` 纯文本，长命令返回 `{"structured": {...}}` 含 task_id + events | 充分 — proxy 层 `handle_tool_call` 需根据结果类型区分输出格式 |
| P4 | MCP Instructions 强引导：更新 instructions 为 "NEVER use raw shell tools. ALL commands go through arshy_run." | 充分 — 改字符串 |
| P5 | Tool description 优化：arshy_run description 明确 "Works for ALL commands." | 充分 — 改字符串 |
| P6-P10 | 接口预留（sandbox_mode, task/stdin, store.backend, middleware, stream output） | 充分 — 只定义 trait/struct stub，不实现功能 |
| P11 | 接入测试：验证 auto 模式切换、短命令纯文本、长命令结构化 | 🧭 探索性 — 需 E2E 测试方案，可能需要 mock daemon 或集成测试框架 |

**产品对齐检查**：
- ✅ "短命令零开销"是核心差异化价值（对比原生 Bash tool 无额外开销）
- ✅ 接口预留不增加复杂度，只定义骨架
- ⚠️ P1 判定规则中有 `--watch`、`serve` 等硬编码 flag——这是启发式方法，后续可由 Skill 提供更精确的 `parse_hint`（对应 Stage S）

---

### Stage S — CLI+Skill 适承

**当前状态**：只有专用 TOML-based parser（20 内置），没有通用 JSON parser，没有 parse_hint 参数。stderr 识别部分由 parser engine 处理（stderr → severity=warning fallback），但无通用错误模式识别。MCP 工具仍是 5 个（arshy_run/query/list/kill/tail）。

**关键文件**：
- `src/daemon/parser/mod.rs` — Parser Engine
- `src/daemon/parser/toml.rs` — 专用 parser
- `src/ipc/mod.rs:138-148` — RunTaskParams
- `src/proxy/mod.rs:294-303` — mcp_tool_to_ipc_method()
- `src/mcp/instructions.rs` — tool_definitions()

**要做的事**：

| Step | 内容 | 探索深度 |
|------|------|----------|
| S1 | 通用 JSON parser：检测 stdout 首字符，JSON 解析 → 结构化事件 | 充分 — 逻辑简单，ROADMAP 中已给出伪代码 |
| S2 | parse_hint 参数：`RunTaskParams` 新增 `parse_hint: Option<String>`，Agent 携带 `"json"` / `"csv"` / `"raw"` | 充分 — 字段定义 + parser engine 路由逻辑 |
| S3 | stderr 通用错误识别：增强 stderr 检测（error:, Error:, FAILED, fatal:, panic! 等模式） | 🧭 探索性 — 通用性 vs 误报率的平衡需实验。当前已有 stderr→warning fallback，需在此之上叠加模式匹配 |
| S4 | mode:auto + parse_hint 联动：短命令若带 `parse_hint="json"` 走零开销 + JSON 解析 | 充分 — 依赖 P2 + S1 完成 |
| S5 | 2 工具模型：`arshy_exec`（action: run/kill/list/tail）+ `arshy_query`，直接切，不做 deprecated 过渡 | 🧭 探索性 — 需验证 2 工具模型在实际 Agent 使用中的 token 节省效果和选择准确率。Tool description 设计是关键，需平衡通用性和可发现性 |
| S6 | CLI+Skill 适承测试：gh/docker/kubectl 等真实 CLI 输出解析 | 🧭 探索性 — 需要收集多种 CLI 的实际输出样本作为 fixture |

**产品对齐检查**：
- ✅ 通用 JSON parser 承接 Skill 驱动的任意 CLI（核心定位）
- ✅ 2 工具模型减少 60% token——Agent 不需要思考"该用哪个工具"
- ✅ 不做 deprecated 过渡——全局原则"无兼容包袱"
- ⚠️ S3 stderr 识别需注意：过度匹配会导致误报，需设定高置信度阈值
- ⚠️ S5 2 工具模型是一次性切换，需在切换前确保 Agent 引导充分（P4 instructions 是前置依赖）

---

## Phase 2: 生产可靠性（P1）

> 目标：做完这两步，arshy 可以部署到生产环境长期运行。
> 不做 = 手动管理 daemon 也可用，但不可靠。

### Stage K — Daemon 生命周期管理

**当前状态**：daemon 通过 `arshyd` 二进制启动，无 PID file、无 start/stop/restart 子命令、无 shutdown RPC、无 stale socket 清理。

**关键文件**：
- `src/daemon/main.rs` — daemon 入口
- `src/cli/mod.rs` — CLI 子命令路由

**要做的事**：

| Step | 内容 | 探索深度 |
|------|------|----------|
| K1 | PID file：`~/.local/share/arshy/arshyd.pid`，防重复启动 | 充分 |
| K2 | `arshy daemon start/stop/restart` CLI 子命令 | 充分 — clap 已有 10 子命令，新增 1 个即可 |
| K3 | `daemon/shutdown` RPC 方法：优雅关闭，通知所有连接 | 充分 — IPC 协议已有通知机制，新增一个 method |
| K4 | Stale socket 清理：启动时检查 UDS 是否对应存活进程 | 🧭 探索性 — 跨平台兼容（macOS/Linux），需处理 PID 回用（PID reuse）问题 |
| K5-K6 | macOS launchd plist / Linux systemd unit | 充分 — 静态模板文件，可选 |
| K7 | 生命周期测试 | 🧭 探索性 — 需测试跨进程启停、信号处理，超出当前单元测试范围 |

**产品对齐检查**：
- ✅ "装一次，之后完全无感"（VISION.md 第二层）

---

### Stage L — 结构化错误处理

**当前状态**：错误通过 `ArshyError` enum 传递，JSON-RPC 错误码未标准化，error.data 字段不丰富，无可重试标记。

**关键文件**：
- `src/error.rs` — 错误类型定义
- `src/ipc/mod.rs:43-54` — ErrorResponse + JsonRpcError（无 data 字段）
- `src/proxy/mod.rs:318-334` — write_json_error（不区分 error code）

**要做的事**：

| Step | 内容 | 探索深度 |
|------|------|----------|
| L1 | JSON-RPC error code 规范：-32600/-32601/-32602/-32603 标准化 | 充分 |
| L2 | error.data 字段：附加 task_id, parser_name, original_command | 充分 — 需在 `JsonRpcError` 中新增 `data` 字段 |
| L3 | 重试语义标记：timeout→可重试，invalid_params→不可重试 | 🧭 探索性 — 需定义完整的可重试性判断矩阵，覆盖所有错误类型 |
| L4 | Proxy 错误映射：IPC error → MCP error 正确转换 | 充分 — 映射关系明确 |
| L5 | 错误测试 | 充分 — 逐 code 覆盖 |

**产品对齐检查**：
- ✅ "Agent 不需要思考该用哪个工具"→ Agent 也不该猜测错误是否可重试

---

## Phase 3: 长期运行（P2）

> 目标：做完这一步，arshy 长期运行不泄漏、可诊断。

### Stage M — 数据完整性 + 可观测

**当前状态**：prune 需手动调用，无 auto-prune、无 WAL checkpoint、无 schema migration、无 stats CLI。

**关键文件**：
- `src/daemon/store/prune.rs` — prune 逻辑
- `src/daemon/store/schema.rs` — schema 初始化
- `src/cli/mod.rs` — CLI 路由

**要做的事**：

| Step | 内容 | 探索深度 |
|------|------|----------|
| M1 | Auto-prune：daemon 启动时自动执行 `prune_older_than_days` | 充分 — 已有 config `auto_prune` 字段和 prune 逻辑 |
| M2 | WAL checkpoint：定期 `PRAGMA wal_checkpoint(TRUNCATE)` | 🧭 探索性 — 需确定触发策略（定时器？连接数阈值？文件大小阈值？） |
| M3 | Schema migration：`schema_version` 表 + 版本递增自动 ALTER TABLE | 充分 — 需定义 migration 格式和当前版本 |
| M4 | `arshy stats` CLI：P50/P99/失败率/按 parser 分组 | 充分 — SQL 聚合查询 |
| M5 | 资源监控：活跃 task 数、SQLite 文件大小、UDS 连接数 | 🧭 探索性 — 是否需要暴露为 MCP resource？是否只是 CLI status 的扩展？ |
| M6 | 完整性测试 | 充分 |

---

## Phase 4: 生态完善（P3）

> 目标：功能增强，但不阻塞核心价值交付。依赖解锁后执行。

### Stage N — MCP 协议完善

| Step | 内容 | 探索深度 |
|------|------|----------|
| N1 | `resources/list` + `resources/read`：暴露 task events 为 MCP resource | ✅ — `arshy://task/{id}` URI + proxy 转发到 daemon IPC |
| N2 | `notifications/cancelled` 处理：Agent 取消 → arshy_kill | ✅ — proxy 已支持 |
| N3 | prompts/list + prompts/get（可选） | 🧭 探索性 — 实际价值不明确，可能低优先级 |
| N4 | MCP 协议版本协商 | 充分 |

### Stage O — Parser 生态解锁

| Step | 内容 | 探索深度 |
|------|------|----------|
| O1 | 解锁 `rhai`：Tier 2 stateful parser（需网络下载依赖） | 🧭 探索性 — 需评估 rhai 嵌入的性能开销和 API 设计。当前 regex+state-machine 回退已可用，rhai 是提升不是必需 |
| O2 | 解锁 `notify` v7：Parser 文件热重载（需网络下载依赖） | 🧭 探索性 — 需确认在 macOS/Linux 上的行为一致性 |
| O3 | 自定义 parser 文档 | 充分 — 用户指南 |
| O4 | Parser 测试扩展 | 充分 |

---

## 依赖解锁时机

| 依赖 | 用途 | 建议时机 |
|------|------|----------|
| `notify` v7 | Parser 文件热重载 | Phase 4 开始时处理，当前 polling 可用 |
| `rhai` | Tier 2 脚本引擎 | Phase 4 开始时处理，当前 regex 回退可用 |

这两个依赖不阻塞 P0/P1/P2 的任何 Stage。

---

## 产品核心约束（每次执行前检查）

1. **无兼容包袱**：任何接口变更直接切换，不做 deprecated 过渡
2. **短命令零开销**：Agent 用 arshy 和用原生 Bash tool 一样快
3. **结构化是增量价值**：不是用结构化输出替代文本输出，而是在需要时提供
4. **arshy 只管执行和理解输出**：不管 Agent 怎么学 CLI（那是 Skill 的事）
5. **安全不可绕过**：即使 permissive 模式，command filter 在代码中强制检查
6. **所有改动必须保持 0 clippy 0 compiler warning**

---

## 完成记录

### Phase 1 (Agent 可接入) ✅
- **Stage J**: 通知实时性 — proxy select 双向监听 + notification batching
- **Stage P**: Agent 无缝接入 — mode:auto 智能推断 + 短命令零开销 + 接口预留
- **Stage S**: CLI+Skill 适承 — JSON parser + parse_hint + stderr 识别 + 2 工具模型

### Phase 2 (生产可靠性) ✅
- **Stage K**: Daemon 生命周期 — PID file + start/stop/restart CLI + shutdown RPC + stale socket cleanup
- **Stage L**: 结构化错误处理 — JSON-RPC error codes + error.data + retryable 标记

### Phase 3 (长期运行) ✅
- **Stage M**: 数据完整性 + 可观测 — auto-prune + WAL checkpoint + schema migration + arshy stats CLI

### Phase 4 (生态完善) ✅
- **Stage N**: MCP 协议完善 — N1 resources/list+read ✅, N2 notifications/cancelled ✅, N3/N4 可选
- **Stage O**: Parser 生态解锁 — rhai v1.24 脚本引擎 ✅, notify v7 热重载 ✅

### 补强 (2026-05-15) ✅
- **rhai 版本锁定**: `rhai = "1.24"` 确保构建可重复性
- **`.rhai` 示例 parser**: `parsers/examples/docker.rhai` + `kubectl.rhai`，用户可参考创建自定义 stateful parser
- **Parser 测试 fixture**: 新增 eslint/go/python/webpack 4 组 fixture（.txt + .json），总计 7/20 parser 有 fixture 覆盖
- **MCP resources**: `resources/list` 暴露最近 50 个 task 为 MCP resource，`resources/read` 返回 task events JSON

---

> **本文件原则**：
> - 标记为 🧭 的步骤不需要在此详述方案，留给执行 agent 自行研究最佳实现路径
> - 标记为"充分"的步骤已包含足够信息供 agent 直接执行
> - 每次完成一个 Stage 后更新本文件的现状描述
> - 策略随代码演进持续修订
