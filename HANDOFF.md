# Arshy 项目交接文档

> 新对话直接粘贴以下 prompt 启动。

---

## 启动 Prompt（直接复制粘贴）

```
Continue from where the project left off. Read HANDOFF.md in the project root for full context, then begin implementing Stage I (Security).

Key rules:
1. No backward compatibility — always use latest interfaces
2. Every change must pass: cargo build + cargo clippy + cargo test
3. All new code must have tests
4. Follow existing code patterns exactly
```

---

## 项目状态

| 指标 | 值 |
|------|------|
| 总测试数 | **153** |
| Library tests | 29 |
| Daemon tests (含 ipc_handler 16) | 115 |
| Proxy tests | 9 |
| Clippy warnings | **0** |
| Compiler warnings | **0** |
| 当前分支 | `main` (ahead of origin by 6 commits) |

## 已完成 Stage

A (基础设施) ✅ → B (Daemon 核心) ✅ → C (Proxy+MCP) ✅ → D (Parser 引擎) ✅ → E (质量) ✅ → F (CLI 命令) ✅ → G (集成测试) ✅ → H (Transport+Proxy 测试) ✅

## 下一步执行顺序

```
Stage I  (安全边界)          ← P0, 当前应执行
Stage J  (通知实时性)        ← P0
Stage P  (Agent 无缝接入)    ← P0
Stage S  (CLI+Skill 适承)    ← P0
Stage K  (生命周期)          ← P1
Stage L  (错误处理)          ← P1
Stage M  (数据完整性)        ← P2
Stage N  (MCP 完善)          ← P3
Stage O  (Parser 解锁)       ← P3
```

## Stage I: 安全边界（当前任务）

| Step | 内容 |
|------|------|
| I1 | **命令过滤引擎** — config 白名单/黑名单 (正则匹配)、默认 blocked 命令集 (rm -rf /, curl\|sh, dd 等) |
| I2 | **路径沙箱** — 限制 cwd 只能在项目目录内，config `sandbox_paths` 配置 |
| I3 | **权限分级** — `read-only` (只能 query/tail) / `full` (可 run/kill)，per-tool 权限检查 |
| I4 | **审计日志** — 所有执行命令独立写入 `~/.local/share/arshy/audit.log`，不受 prune 影响 |
| I5 | **安全测试** — 白名单命中、黑名单拦截、路径逃逸拒绝、权限拒绝 |

### I1 实现要点

- 新建 `src/daemon/security/mod.rs` + `filter.rs` + `audit.rs`
- Filter 在 `Executor::run()` 入口处拦截
- config 新增 `[security]` 节: `allowed_commands`, `blocked_patterns`, `sandbox_paths`, `audit_log`
- 默认 blocked: `rm -rf /`, `rm -rf ~`, `curl|sh`, `wget|sh`, `dd if=`, `mkfs`, `:(){ :|:& };:`

### I2 实现要点

- `sandbox_paths` 默认为 cwd
- run 时检查 `params.cwd` 是否在 `sandbox_paths` 内
- 路径逃逸: `../` 或 symlink → 拒绝

### I3 实现要点

- config `security.access_level: "full" | "read-only"`
- read-only 模式: `task/run` 和 `task/kill` 返回 -32603 权限拒绝
- per-tool 检查在 ipc_handler dispatch 中

### I4 实现要点

- 独立文件 `~/.local/share/arshy/audit.log`，append-only
- 记录: timestamp, command, cwd, user, exit_code, task_id
- 不受 prune 影响（独立于 SQLite）

## 关键文件路径

| 文件 | 作用 |
|------|------|
| `src/daemon/exec/mod.rs` | Executor — run/kill/tail 入口 |
| `src/daemon/ipc_handler.rs` | JSON-RPC dispatch |
| `src/daemon/store/mod.rs` | SQLite 存储 |
| `src/daemon/bus/mod.rs` | EventBus |
| `src/ipc/mod.rs` | IPC 类型定义 |
| `src/mcp/instructions.rs` | MCP tool 定义 |
| `src/proxy/mod.rs` | MCP proxy |
| `src/cli/mod.rs` | CLI 子命令 |
| `Cargo.toml` | 双 binary: arshy + arshyd |

## 全局原则

1. **无兼容包袱** — 直接用最新接口，不做 deprecated 过渡
2. **测试覆盖** — 新增代码必须有对应测试
3. **零 warning** — cargo build + clippy 必须 0 errors/0 warnings
4. **现有模式** — 新模块遵循 store/parser/bus 的目录结构和命名风格

## 项目架构速览

```
Agent ←MCP(stdio)→ Proxy ←UDS→ Daemon
                                  ├─ Executor (PTY spawn + kill)
                                  ├─ Parser Engine (20 TOML + crash + raw)
                                  ├─ Store (SQLite WAL)
                                  └─ EventBus → NotificationRouter
```

## 文档

- `VISION.md` — 产品愿景
- `ROADMAP.md` — 完整路线图（含 Stage I-O + S）
- `HANDOFF.md` — 本文件
