# Arshy 开发路线图

> **AI Agent 唯一的 Shell。** — 所有 shell 操作通过 arshy 执行，原生 MCP 协议，结构化输出，安全沙箱。

### 目标定位

Arshy 不是 Agent 的"可选增强"，而是 Agent 执行 shell 的**唯一通道**。

**全局原则：无兼容包袱，始终用最新接口。**
arshy 尚未正式发布，没有存量用户。所有接口变更直接切换，不做 deprecated 过渡、不做兼容层。Stage S 将 MCP 工具从 5 个直接切为 2 个，不留旧接口。

**子定位：CLI+Skill 执行基座。**
未来 Agent 生态通过动态加载 Skill 学习如何使用人类 CLI 工具，而 Skill 驱动的每一条命令都必须经过 shell。Arshy 就是这条必经之路——Skill 管"跑什么"，arshy 管"怎么跑"和"理解输出"。

```
Agent 加载 Skill → 知道用哪个 CLI → 执行命令 → arshy (shell) → 结构化输出
       ↑                                        ↑
   不是 arshy 的职责                        arshy 的全部职责
```

| 对比 | 原生 Bash tool | Arshy |
|------|---------------|-------|
| 输出 | 原始文本，Agent 需自己解析 | 结构化事件 (error/warning/file/line) |
| 错误定位 | 无，Agent 在文本中搜索 | 精确到 文件:行:列 + ±3 行源码上下文 |
| 长任务 | 阻塞等待或超时丢弃 | async + 实时通知 + 优雅终止 |
| 安全 | 无限制 | 命令过滤 + 路径沙箱 + 权限分级 + 审计 |
| 持久化 | 无 | SQLite 跨会话查询历史 |
| 短命令 | 直接执行 | 同样直接执行，零额外开销 |
| 通用 CLI | Agent 自己处理任意输出 | 通用 JSON parser + stderr 错误识别 + parse_hint，任意 CLI 输出均可结构化 |

---

## Stage A: 基础设施 ✅

| Step | 内容 | 状态 |
|------|------|------|
| A1 | 项目骨架：双 Target、完整目录结构、依赖清单 | ✅ |
| A2 | 配置系统：TOML + XDG + 四层合并 (CLI>Env>File>Default) | ✅ |
| A3 | 错误类型 + 日志：thiserror + tracing | ✅ |

---

## Stage B: Daemon 核心 ✅

| Step | 内容 | 状态 |
|------|------|------|
| B1 | **SQLite 存储** — 4 表 (tasks/events/tool_versions/parser_registry)、WAL、CRUD、prune、integrity check | ✅ 29 tests |
| B2 | **PTY 执行引擎** — spawn、stdout/stderr 流式读取、Task 状态机、graceful kill (SIGINT→SIGTERM→SIGKILL) | ✅ 7 tests |
| B3 | **Daemon 主循环** — UDS listener、多连接管理、JSON-RPC 路由、优雅关闭 | ✅ |
| B4 | **IPC 协议** — JSON Lines 帧、DaemonConnection 双向通道、60s 超时、proxy 自动重连 | ✅ |

---

## Stage C: Proxy + MCP ✅

| Step | 内容 | 状态 |
|------|------|------|
| C1 | **MCP 协议** — JSON-RPC 2.0、Capabilities、Instructions | ✅ |
| C2 | **Proxy 网关** — stdin/stdout MCP server、UDS 连接 daemon、请求转发 | ✅ |
| C3 | **通知推送** — EventBus + NotificationRouter、diagnostic/update/complete/shutdown 4 类通知 | ✅ 11 tests |
| C4 | **5 工具联调** — arshy_run / query / list / kill / tail 全链路 | ✅ |

---

## Stage D: Parser 引擎 ✅

| Step | 内容 | 状态 |
|------|------|------|
| D1 | **Tier 1 TOML** — 行正则匹配、37 内置 parser (tsc/cargo/jest/vite/eslint/go/python/cc/npm/webpack/prettier/swc/esbuild/clippy/make/gradle/cargo-test/mocha/pip/pnpm/kubectl/docker/terraform/helm/aws/uv/ruff/turbo/nx/deno/bun/biome/oxlint/vitest/curl/git/ssh) | ✅ 26 tests |
| D2 | **Parser 加载** — `include_str!` 编译入二进制 + 文件系统加载 | ✅ |
| D3 | **版本探测** — tool_versions SQLite 缓存、15 工具支持、24h TTL | ✅ |
| D4 | **Crash Parser** — 通用崩溃/traceback 检测 (Go/Python/Rust/Node/Shell) | ✅ 8 tests |
| D5 | **状态解析器** — regex+state-machine 回退实现 (npm/webpack 等多行输出) | ✅ |
| D6 | **Test Harness** — fixture 格式 (.txt/.json)、匹配率 ≥95% 阈值、37 parser 全覆盖 | ✅ |

---

## Stage E: 质量 & 健壮性 ✅

| Step | 内容 | 状态 |
|------|------|------|
| E1 | **Store 测试** — tasks/events/prune/versions/schema 全覆盖 | ✅ 29 tests |
| E2 | **Bus/Router 测试** — EventBus 发布/订阅 + 4 类通知映射 | ✅ 11 tests |
| E3 | **Config 测试** — merge_partial/env/cli/路径展开/parse_bool | ✅ 19 tests |
| E4 | **Parser 健壮性** — `catch_unwind` 防 panic、截断 warning event、SQLite integrity check | ✅ |
| E5 | **IPC 健壮性** — send_request 60s 超时、proxy 连接断开自动重连 | ✅ |

---

## Stage F: CLI 管理命令 ✅

| Step | 内容 | 状态 |
|------|------|------|
| F1 | 10 子命令路由 (run/list/query/kill/tail/install/uninstall/prune/config/status) | ✅ |
| F2 | arshy install / uninstall（Claude Code 注册） | ✅ |
| F3 | arshy config get/set/list/path（27 key 动态导航） | ✅ |
| F4 | arshy prune / status | ✅ |

---

## Stage G: 集成测试 + E2E ✅

| Step | 内容 | 状态 |
|------|------|------|
| G1 | IPC Handler 7 方法集成测试 (UnixStream::pair) | ✅ 16 tests |
| G2 | 通知流 E2E — task/update、task/complete、diagnostic | ✅ |
| G3 | 错误路径 — 未知方法、缺参数、畸形 JSON、幂等 kill | ✅ |
| G4 | 并发 — 串行 5 task、双连接并行、client 断开、分页 | ✅ |

## Stage H: Transport + Proxy 测试 ✅

| Step | 内容 | 状态 |
|------|------|------|
| H1 | DaemonConnection 测试 — request/response、timeout、notification、drain | ✅ 10 tests |
| H2 | Proxy helper 测试 — mcp_tool_to_ipc_method、is_connection_error | ✅ 2 tests |
| H3 | MCP 通知映射 — task/update、task/complete、diagnostic、shutdown、unknown | ✅ 7 tests |

---

## Stage I: 安全边界 ✅

> P0 — 不做此项，AI Agent 无法安全接入。

| Step | 内容 | 状态 |
|------|------|------|
| I1 | **命令过滤引擎** — config 白名单/黑名单 (正则匹配)、默认 blocked 命令集 (rm -rf /, curl\|sh, dd 等) | ✅ 22 tests |
| I2 | **路径沙箱** — 限制 cwd 只能在项目目录内，config `sandbox_paths` 配置 | ✅ 8 tests |
| I3 | **权限分级** — `read-only` (只能 query/tail) / `full` (可 run/kill)，per-tool 权限检查 | ✅ 6 tests |
| I4 | **审计日志** — 所有执行命令独立写入 `~/.local/share/arshy/audit.log`，不受 prune 影响 | ✅ 9 tests |
| I5 | **安全测试** — 白名单命中、黑名单拦截、路径逃逸拒绝、权限拒绝 | ✅ 17 e2e tests |

---

## Stage J: 通知实时性 ✅

> P0 — 不做此项，async 模式通知不可用。

| Step | 内容 | 状态 |
|------|------|------|
| J1 | **Proxy 双向 select** — 主循环同时 select stdin + notification channel，任意就绪即处理 | ✅ |
| J2 | **Notification forwarder task** — 独立后台 task 从 daemon channel 读取并写入 stdout | ✅ |
| J3 | **通知缓冲** — 高频事件 (多 task 并行) 合并 batch 发送，config `batch_interval_ms` | ✅ |
| J4 | **通知测试** — 验证 async 模式下 Agent 发 run 后能即时收到 update/complete | ✅ |

---

## Stage P: Agent 无缝接入 ✅

> P0 — 做完此项，arshy 成为 Agent 的唯一 shell 通道。

### Step 列表

| Step | 内容 | 状态 |
|------|------|------|
| P1 | **mode:auto 智能推断** — 短命令 (≤5 词、无管道、无长时间运行标志) → sync + raw text + 跳过 Store；其余 → sync 60s + 结构化 + Store | ✅ |
| P2 | **短命令零开销路径** — `executor.run()` 中 auto 模式判断：短命令不 insert_task、不 spawn parser session、直接 spawn → wait → return stdout 原文 | ✅ |
| P3 | **输出格式自动切换** — MCP response: 短命令返回纯文本，长命令返回结构化 (task_id + status + events) | ✅ |
| P4 | **MCP Instructions 强引导** — instructions: "NEVER use raw shell tools. ALL commands go through arshy_exec." | ✅ |
| P5 | **Tool description 优化** — arshy_exec description: "Works for ALL commands. Short commands return instantly." | ✅ |
| P6 | **接口预留: sandbox_mode** — config `executor.sandbox_mode: "none" | "process" | "container"`，executor 中 stub 分支 | ✅ |
| P7 | **接口预留: task/stdin** — IPC 方法定义 + executor stub（返回 "stdin write not supported yet"） | ✅ |
| P8 | **接口预留: store.backend** — config `store.backend: "sqlite" | "postgres" | "redis"`，Store 当前只实现 SQLite | ✅ |
| P9 | **接口预留: proxy middleware** — `proxy::Middleware` trait 定义，当前为空 Vec | ✅ |
| P10 | **接口预留: notifications/stream** — EventBus 新增 `StreamOutput` variant，当前不产生此事件 | ✅ |
| P11 | **接入测试** — 验证: 短命令返回纯文本、长命令返回结构化、mode:auto 自动切换 | ✅ |

---

## Stage K: Daemon 生命周期管理 ✅

> P1 — 不做此项，生产部署不可靠。

| Step | 内容 | 状态 |
|------|------|------|
| K1 | **PID file** — `~/.local/share/arshy/arshyd.pid`，防止重复启动 | ✅ |
| K2 | **`arshy daemon start/stop/restart`** — CLI 子命令 | ✅ |
| K3 | **Daemon shutdown RPC** — `daemon/shutdown` 方法，优雅关闭并通知所有连接 | ✅ |
| K4 | **Stale socket 清理** — 启动时检查旧 UDS 文件是否对应存活进程 | ✅ |
| K5 | **macOS launchd plist** — `arshy install-launchd` CLI 安装 `~/Library/LaunchAgents/com.arshy.daemon.plist` | ✅ |
| K6 | **Linux systemd unit** — `arshy install-systemd` CLI 安装 `~/.config/systemd/user/arshyd.service` | ✅ |
| K7 | **生命周期测试** — 重复启动拒绝、stop 后连接断开、stale pid 恢复 | ✅ |

---

## Stage L: 结构化错误处理 ✅

> P1 — 不做此项，Agent 无法智能重试。

| Step | 内容 | 状态 |
|------|------|------|
| L1 | **JSON-RPC error code 规范** — -32600 invalid request, -32601 method not found, -32602 invalid params, -32603 internal error | ✅ |
| L2 | **error.data 字段** — 附加 task_id, parser_name, original_command 等上下文 | ✅ |
| L3 | **重试语义标记** — error 中标记是否可重试 (timeout=true, invalid_params=false) | ✅ |
| L4 | **Proxy 错误映射** — IPC error → MCP error 正确转换，保留 code + message + data | ✅ |
| L5 | **错误测试** — 覆盖所有 error code 路径 | ✅ |

---

## Stage M: 数据完整性 + 可观测 ✅

> P2 — 不做此项，长期运行不可靠。

| Step | 内容 | 状态 |
|------|------|------|
| M1 | **Auto-prune** — daemon 启动时自动执行 `prune_older_than_days` | ✅ |
| M2 | **WAL checkpoint** — 定期 `PRAGMA wal_checkpoint(TRUNCATE)` 防 WAL 膨胀 | ✅ |
| M3 | **Schema migration** — `schema_version` 表，版本递增时自动 ALTER TABLE | ✅ |
| M4 | **`arshy stats` CLI** — 平均执行时间、P99、失败率、按 parser 分组统计 | ✅ |
| M5 | **资源监控** — 活跃 task 数、SQLite 文件大小、UDS 连接数 | ✅ |
| M6 | **完整性测试** — migration 升级、WAL checkpoint、auto-prune 触发 | ✅ |

---

## Stage N: MCP 协议完善 ✅

> P3 — 不做此项，功能非核心但提升 Agent 体验。

| Step | 内容 | 状态 |
|------|------|------|
| N1 | **`resources/list` + `resources/read`** — 暴露 task 事件为 MCP resource | ✅ |
| N2 | **`notifications/cancelled` 处理** — Agent 取消任务信号 → 调用 `arshy_kill` | ✅ |
| N3 | **`prompts/list` + `prompts/get`** — analyze_build_failure / diagnose_test_failure / review_task_output | ✅ |
| N4 | **MCP 协议版本协商** — client/server protocol version 对齐检查 | ✅ |

---

## Stage O: Parser 生态解锁 ✅

> P3 — 不做此项，parser 能力降级但可用。

| Step | 内容 | 状态 |
|------|------|------|
| O1 | **解锁 `rhai`** — Tier 2 stateful parser 完整实现，替代 regex+state-machine 回退 | ✅ |
| O2 | **解锁 `notify` v7** — Parser 文件热重载，修改无需重启 daemon | ✅ |
| O3 | **自定义 parser 文档** — guides/custom-parser-toml.md + custom-parser-rhai.md | ✅ |
| O4 | **Parser 测试扩展** — 37/37 parser 有 fixture 覆盖 (匹配率 ≥95%) | ✅ |

---

## Stage S: CLI+Skill 适承 ✅

> P0 — 做完此项，arshy 能承接 Skill 驱动的任意 CLI 执行。

| Step | 内容 | 状态 |
|------|------|------|
| S1 | **通用 JSON/格式 parser** — JSON、NDJSON、YAML、CSV/TSV 自动检测解析 | ✅ |
| S2 | **parse_hint 参数** — `RunTaskParams.parse_hint: Option<String>`，Agent 携带 `"json"` / `"csv"` / `"raw"` hint | ✅ |
| S3 | **stderr 通用错误识别** — crash parser 覆盖 Go/Python/Rust/Node/Shell 崩溃模式 | ✅ |
| S4 | **mode:auto + parse_hint 联动** — hint 可绕过短命令路径，强制结构化输出 | ✅ |
| S5 | **2 工具模型** — `arshy_exec`（action: run/kill/list/tail/cd/subscribe）+ `arshy_query`，减少 ~60% tool 定义 token | ✅ |
| S6 | **CLI+Skill 适承测试** — 覆盖 JSON/CSV/YAML 输出解析、parse_hint 路由、stderr 识别 | ✅ |

---

## Stage T: 竞品功能对标 ✅

> P0 — 借鉴 headroom/RTK 的优秀实践，强化 arshy 的解析深度和可靠性。

| Step | 内容 | 状态 |
|------|------|------|
| T1 | **启发式错误过滤器** — tier 4.5，关键词检测 (error/fatal/FAILED/panic/traceback) + file:line:col 提取，未匹配工具也能识别错误 | ✅ 10 tests |
| T2 | **事件去重** — 连续相同事件折叠为 "(repeated N times)"，减少 token 浪费 | ✅ 6 tests |
| T3 | **Tee 失败恢复** — 全量原始输出存入 SQLite (raw_output 列)，任务失败时可通过 `tail --format raw` 取回 | ✅ 3 tests |
| T4 | **Errors-only 模式** — `--errors-only` CLI 参数 + MCP 参数，只返回 error 级别事件 | ✅ 3 tests |
| T5 | **Docker/kubectl/AWS 解析器** — 3 个新 TOML parser，覆盖容器和云 CLI | ✅ 100% 准确率 |
| T6 | **Token 节省统计** — `arshy stats` 显示 token 节省百分比和 parser 覆盖率 | ✅ 1 test |
| T7 | **多 AI 工具集成文档** — Cursor/Codex/Copilot/Gemini 集成指南 | ✅ |

---

## Stage U: 上下文丰富 ✅

> P0 — 让 Agent 从"读日志"变成"懂问题"。

| Step | 内容 | 状态 |
|------|------|------|
| U1 | **源码上下文丰富** — 错误事件自动附带 ±3 行源码，文件缓存避免重复读取 | ✅ 6 tests |
| U2 | **Git 变更关联** — 错误文件与 `git diff --name-only HEAD~1` 交叉比对，标记 `recently_changed` | ✅ 4 tests |
| U3 | **Executor 集成** — `spawn_blocking` 包装避免阻塞异步运行时，enrichment 在 store 查询后、summary 计算前执行 | ✅ |

---

## Stage V: 错误码诊断 ✅

> P0 — 让 Agent 从"看到错误"变成"知道怎么修"。

| Step | 内容 | 状态 |
|------|------|------|
| V1 | **EventHint 结构** — TaskEvent 新增 hint 字段 (cause, fix, retry) | ✅ |
| V2 | **HintDb 加载** — parsers/errors/*.toml 编译时加载，LazyLock 单例 | ✅ 7 tests |
| V3 | **4 语言精选错误码** — TypeScript (4), Python (2), Go (1) 共 7 个（仅保留 LLM 不易识别的） | ✅ |
| V4 | **Executor 集成** — sync 路径自动查表附加 hint | ✅ |

---

## 下一阶段: 补全 & 产品化

> 所有 ROADMAP Stage 代码已完成。以下为剩余缺口和产品化方向。

### 缺口: Parser Fixture 覆盖

**已补全。** 37/37 builtin parser 均有 fixture 测试覆盖。

### 缺口: 3 个被忽略的集成测试

`tests/integration.rs` 中 3 个测试需无运行 daemon 环境。功能已被 ipc_handler 集成测试覆盖。

### 产品化方向

| 方向 | 说明 |
|------|------|
| 分发 | Homebrew formula、cargo install、npm 包装、GitHub Release 自动化 |
| Parser 生态扩展 | biome, turbopack, rspack, oxlint, systemctl, brew 等新兴/系统工具 |
| 沙箱执行 | sandbox_mode "process" (seccomp-bpf / sandbox-exec) / "container" (Docker) |
| 多存储后端 | store.backend "postgres" — 企业多实例场景 |
| 中间件系统 | Middleware trait 实现: AuditMiddleware, RateLimitMiddleware, AuthMiddleware |
| 真实环境验证 | Agent 日常使用反馈、mode:auto 准确率、parser 匹配率、通知延迟评估 |

---

## 待解锁依赖

| 依赖 | 用途 | 状态 |
|------|------|------|
| `notify` v7 | Parser 文件热重载 (替代 polling) | ✅ 已解锁 |
| `rhai` | Tier 2 脚本引擎 (替代 regex+state-machine 回退) | ✅ 已解锁 |

---

## 测试统计

| 指标 | 值 |
|------|------|
| 总测试数 | **363** |
| Library tests | 31 |
| Proxy tests | 23 |
| Daemon tests | 299 |
| 被忽略测试 | 3 (integration, 需无运行 daemon 环境) |
| Clippy warnings | **0** |
| Compiler warnings | **0** |
| Builtin parsers | **37** |
| Parser fixtures | **37/37** (匹配率 ≥95%) |

---

## 图例

| 符号 | 含义 |
|------|------|
| ✅ | 已完成（含测试） |
| 🟡 | 骨架就绪（接口定义完成，需接线） |
| 🔵 | 待解锁（依赖或外部条件限制） |
| ⬜ | 未开始 |

---

## 优先级说明

| 优先级 | Stage | 说明 |
|--------|-------|------|
| **P0** | I (安全), J (通知实时性), P (Agent 无缝接入), S (CLI+Skill 适承), T (竞品功能对标), U (上下文丰富) | ✅ 全部完成 |
| **P1** | K (生命周期), L (错误处理) | ✅ 全部完成 |
| **P2** | M (数据完整性/可观测) | ✅ 全部完成 |
| **P3** | N (MCP 完善), O (Parser 解锁) | ✅ 全部完成 |

---

## 执行状态

```
Stage A  (基础设施)          ✅
Stage B  (Daemon 核心)       ✅
Stage C  (Proxy + MCP)       ✅
Stage D  (Parser 引擎)       ✅
Stage E  (质量 & 健壮性)     ✅
Stage F  (CLI 管理)          ✅
Stage G  (集成测试)          ✅
Stage H  (Transport 测试)    ✅
Stage I  (安全)              ✅
Stage J  (通知实时性)        ✅
Stage K  (生命周期)          ✅
Stage L  (错误处理)          ✅
Stage M  (数据完整性)        ✅
Stage N  (MCP 完善)          ✅
Stage O  (Parser 解锁)       ✅
Stage P  (Agent 无缝接入)    ✅
Stage S  (CLI+Skill 适承)    ✅
Stage T  (竞品功能对标)       ✅
Stage U  (上下文丰富)         ✅
```

**所有 ROADMAP Stage 已完成。** 剩余工作见"下一阶段: 补全 & 产品化"。
