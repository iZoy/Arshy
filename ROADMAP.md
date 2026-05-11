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

## Stage B: Daemon 核心（进行中）

| Step | 内容 | 状态 |
|------|------|------|
| B1 | **SQLite 存储** — 4 表 (tasks/events/tool_versions/parser_registry)、WAL、CRUD、prune | 🟡 骨架就绪 |
| B2 | **PTY 执行引擎** — portable-pty spawn、stdout/stderr 流式读取、Task 状态机、kill 策略 | ⬜ |
| B3 | **Daemon 主循环** — UDS listener、多连接管理、JSON-RPC 路由、优雅关闭 | 🟡 骨架就绪 |
| B4 | **IPC 协议** — JSON Lines 帧处理、请求/响应/通知 | 🟡 骨架就绪 |

### B2 任务清单（下一个里程碑）

- [ ] `portable-pty` 嵌入 daemon，实现 `spawn_pty()` 真实 spawn
- [ ] 实现 stdout/stderr 流读取，逐行送入 parser 引擎
- [ ] 实现 Task 状态机完整流转：running → completed/failed/killed/timeout
- [ ] 实现 `ProcessManager::kill()`：SIGINT → wait → SIGTERM → wait → SIGKILL
- [ ] 实现 `Executor::tail()`：从 store 读取最新 N 条 events
- [ ] 集成 EventBus 通知：task 状态变化时 publish 事件

---

## Stage C: Proxy + MCP

| Step | 内容 | 状态 |
|------|------|------|
| C1 | **MCP 协议** — JSON-RPC 核心、Capabilities、Instructions | 🟡 骨架就绪 |
| C2 | **Proxy 网关** — stdin/stdout MCP server、UDS 连接 daemon、请求转发 | 🟡 骨架就绪 |
| C3 | **通知推送** — EventBus + NotificationRouter、diagnostic/update/complete 通知 | 🟡 骨架就绪 |
| C4 | **5 工具完整联调** — arshy_run/query/list/kill/tail 全链路 | ⬜ |

---

## Stage D: Parser 引擎

| Step | 内容 | 状态 |
|------|------|------|
| D1 | **Tier 1 TOML** — 行正则匹配、内置 5 parser (tsc/vite/jest/cargo/raw) | ⬜ |
| D2 | **Parser 加载** — 内置编译入二进制 + 文件系统加载 + 热更新 | ⬜ |
| D3 | **版本探测** — tool_versions 缓存、语义版本、24h TTL | ⬜ |
| D4 | **Tier 2 Rhai** — Rhai 引擎嵌入、沙箱、on_line/on_complete 回调 | ⬜ |
| D5 | **Test Harness** — fixture 格式、匹配率计算、质量分判定 | ⬜ |

---

## Stage E: CLI 管理命令（已完成骨架）

| Step | 内容 | 状态 |
|------|------|------|
| E1 | 10 子命令路由 (run/list/query/kill/tail/install/uninstall/prune/config/status) | 🟡 骨架就绪 |
| E2 | arshy install / uninstall（Claude Code 注册） | 🟡 骨架就绪 |
| E3 | arshy config get/set/list | 🟡 骨架就绪 |
| E4 | arshy prune / status | 🟡 骨架就绪 |

---

## 图例

| 符号 | 含义 |
|------|------|
| ✅ | 已完成（含骨架） |
| 🟡 | 骨架就绪（接口定义完成，需接线） |
| ⬜ | 未开始 |
