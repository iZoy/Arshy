# Arshy 开发路线图

> AI Agent 原生 Shell 执行层 — 结构化输出、异步通知、跨会话持久化。

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
| D1 | **Tier 1 TOML** — 行正则匹配、20 内置 parser (tsc/cargo/jest/vite/eslint/go/python/cc/npm/webpack/prettier/swc/esbuild/clippy/make/gradle/cargo-test/mocha/pip/pnpm) | ✅ 26 tests |
| D2 | **Parser 加载** — `include_str!` 编译入二进制 + 文件系统加载 | ✅ |
| D3 | **版本探测** — tool_versions SQLite 缓存、15 工具支持、24h TTL | ✅ |
| D4 | **Crash Parser** — 通用崩溃/traceback 检测 (Go/Python/Rust/Node/Shell) | ✅ 8 tests |
| D5 | **状态解析器** — regex+state-machine 回退实现 (npm/webpack 等多行输出) | ✅ |
| D6 | **Test Harness** — fixture 格式 (.txt/.json)、匹配率 ≥95% 阈值 | ✅ |

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

## 待解锁依赖

| 依赖 | 用途 | 状态 |
|------|------|------|
| `notify` v7 | Parser 文件热重载 (替代 polling) | 🔵 网络问题，待解锁 |
| `rhai` | Tier 2 脚本引擎 (替代 regex+state-machine 回退) | 🔵 待解锁 |

---

## Stage G: 集成测试 + E2E ✅

| Step | 内容 | 状态 |
|------|------|------|
| G1 | IPC Handler 7 方法集成测试 (UnixStream::pair) | ✅ 16 tests |
| G2 | 通知流 E2E — task/update、task/complete、diagnostic | ✅ |
| G3 | 错误路径 — 未知方法、缺参数、畸形 JSON、幂等 kill | ✅ |
| G4 | 并发 — 串行 5 task、双连接并行、client 断开、分页 | ✅ |

---

## 测试统计

| 指标 | 值 |
|------|------|
| 总测试数 | **134** |
| Library tests | 19 |
| Daemon tests | 99 |
| 集成测试 | 16 |
| Clippy warnings | **0** |
| Compiler warnings | **0** |

---

## 图例

| 符号 | 含义 |
|------|------|
| ✅ | 已完成（含测试） |
| 🟡 | 骨架就绪（接口定义完成，需接线） |
| 🔵 | 待解锁（依赖或外部条件限制） |
| ⬜ | 未开始 |
