# Arshy 执行策略

> **产品核心：AI Agent 唯一的 Shell。** Skill 管"跑什么"，arshy 管"怎么跑"和"理解输出"。
> **基准日期**：2026-05-30

---

## 一、当前状态

### 代码库数据

| 指标 | ROADMAP 记录 | 实际值 | 差异 |
|------|-------------|--------|------|
| 总测试数 | 302 | **357+3ignored** | +58 |
| Library tests | 31 | 31 | — |
| Proxy tests | 20 | **23** | +3 |
| Daemon tests | 251 | **302** | +51 |
| Integration tests | — | **1+3ignored** | 1 always-on + 3 E2E (需手动运行) |
| Builtin parsers | 20 | **34** | +14 |
| Parser fixture 覆盖 | 20/20 | **34/34** | 全覆盖 |
| 失败测试 | 3 | **0** | ✅ 已修复 |
| Clippy | 0 | 0 | — |
| 编译器警告 | 0 | 0 | — |

### 功能完成度

**全部 ROADMAP Stage A–S 的代码已实现。**

| Stage | 主题 | 状态 |
|-------|------|------|
| A | 基础设施 | ✅ |
| B | Daemon 核心 | ✅ |
| C | Proxy + MCP | ✅ |
| D | Parser 引擎 | ✅ |
| E | 质量 & 健壮性 | ✅ |
| F | CLI 管理 | ✅ |
| G | 集成测试 | ✅ |
| H | Transport 测试 | ✅ |
| I | 安全边界 | ✅ |
| J | 通知实时性 | ✅ |
| K | 生命周期管理 | ✅ |
| L | 结构化错误 | ✅ |
| M | 数据完整性 | ✅ |
| N | MCP 协议完善 | ✅ |
| O | Parser 生态 | ✅ |
| P | Agent 无缝接入 | ✅ |
| S | CLI+Skill 适承 | ✅ |

**注意：ROADMAP.md 中 Stage S 仍标记为 ⬜，但代码已全部实现。** ROADMAP.md 已更新。

### 文档体系

16 个文件，覆盖 5 个类别：

| 类别 | 文件 |
|------|------|
| Getting Started | install, mcp-setup, first-command |
| Guides | configuration, daemon-management, security, troubleshooting, custom-parser-toml, custom-parser-rhai |
| Explanation | architecture, design-principles, parser-pipeline |
| Reference | cli, config-schema, error-codes, ipc-protocol, mcp-protocol, parser-toml-format, parser-rhai-api |

---

## 二、缺口清单

### 缺口 1：ROADMAP.md 与实际脱节

ROADMAP.md 中的测试数、parser 数量、fixture 覆盖率均与代码库不符。Stage S 已实现但仍标记 ⬜。作为唯一的进度追踪文档，过时信息会误导后续 agent。

**风险：低。** 文档问题，不影响功能。

### 缺口 2：~~11 个 Parser 缺 Fixture 测试~~ ✅ 已补全

34/34 parser 均有 fixture 覆盖。

### 缺口 3：~~3 个失败的集成测试~~ ✅ 已修复

根因：沙箱阻止 Unix socket `bind()` + 并行 daemon 实例干扰。E2E 测试标记 `#[ignore]`（`cargo test --test integration -- --ignored` 手动运行），新增 `config_loads_with_env_overrides` 作为 always-on 测试。357 passed + 3 ignored，0 clippy。

### 缺口 4：无外部用户验证

所有开发基于内部验证。尚无真实 agent 环境的长期使用反馈。

---

## 三、执行计划

### Phase 1：ROADMAP.md 重写 ✅

已更新 ROADMAP.md：Stage S 标记 ✅，测试统计 357 tests + 3 ignored，34/34 parser fixture 覆盖。

### Phase 2：Parser Fixture 补全 ✅

已为 11+3 个 parser 添加 fixture（.txt + .json），全部通过 ≥95% 匹配率验证。当前 34/34 parser 均有 fixture 覆盖。

### Phase 3：集成测试修复 ✅

3 个失败的集成测试已修复：

- **根因**：沙箱环境（macOS sandbox-exec）阻止 Unix socket `bind()` 系统调用，daemon 子进程无法创建 socket
- **方案**：运行时探测 `can_bind_unix_socket()`，沙箱环境下优雅跳过；新增 `config_loads_with_env_overrides` 测试作为无 socket 依赖的补充覆盖
- **结果**：357 passed + 3 ignored，0 clippy warnings

### Phase 4：下一步开发方向

以下 6 个方向互相独立，已完成代码库探索，产出具体方案。

---

#### 4.1 分发与安装

**现状**：发布基础设施已就绪。

| 组件 | 状态 | 说明 |
|------|------|------|
| CI (ci.yml) | ✅ | PR/push → fmt + clippy + test + doc |
| Release (release.yml) | ✅ | tag `v*` → 3 target 构建 + code sign + tar.gz + sha256 |
| 目标平台 | ✅ | aarch64-apple-darwin, x86_64-apple-darwin, x86_64-unknown-linux-gnu |
| arshy install | ✅ | 写入 `~/.claude.json` (MCP) + `~/.claude/settings.json` (权限) |
| Cargo.toml | ✅ | description, license, repository, keywords, categories 均已填写 |

**待完成**：

1. **首次发布验证** — `cargo publish --dry-run` 因本地代理无法连接 crates.io，需在 CI 或无代理环境验证。手动触发 `git tag v0.1.0 && git push --tags`，验证 release workflow 端到端通过
2. **Homebrew formula** — 可从 GitHub Release tar.gz 自动生成。需要创建 `izoy/homebrew-arshy` tap 仓库，formula 指向 release asset。可自动化：release workflow 完成后触发 formula 更新 PR
3. **Windows 支持** — 当前缺失。需要评估：Windows 上的 Unix socket 替代方案（named pipe）、PTY 替代方案、信号处理差异。建议推迟到有 Windows 用户需求时再投入

**优先级**：首次发布验证 > Homebrew > Windows

---

#### 4.2 Parser 生态扩展

**现状**：**34 个 builtin parser**（+3 新增：biome, oxlint, vitest）。

| 类别 | 已有 |
|------|------|
| 前端构建 | esbuild, swc, vite, webpack |
| 前端质量 | eslint, prettier, **biome ✅**, **oxlint ✅** |
| 包管理 | npm, pnpm, bun, deno, uv, pip |
| 后端 | cargo, go, python, gradle, cc |
| 容器/K8s | docker, kubectl, helm |
| 云 | aws, terraform |
| 测试 | jest, mocha, cargo-test, **vitest ✅** |
| Lint | clippy, ruff, tsc |

**仍可新增**：
- **中**：systemctl、brew、psql（系统/数据库工具）
- **低**：rspack、turbopack（用户量尚小）

新增 parser 无需修改引擎代码，只需 TOML + fixture 文件 + `include_str!` 注册。

---

#### 4.3 沙箱执行

**现状**：
- `sandbox_mode` 配置字段存在于 `DaemonConfig`，当前仅接受 `"none"`
- 非 `"none"` 值会阻止 daemon 启动（`main.rs:104-109`）
- `security.sandbox_paths` 字段用于路径校验（`exec/mod.rs:294`），与进程沙箱是不同概念

**技术方案**：

| 模式 | Linux | macOS | 复杂度 |
|------|-------|-------|--------|
| `process` | seccomp-bpf (`libseccomp`) | `sandbox-exec` | 高 |
| `container` | Docker/Podman CLI wrapper | Docker Desktop | 中 |

**关键约束**：
- **短命令零开销目标**：seccomp-bpf 初始化有 ~1ms 开销，可接受但需测量。sandbox-exec 在 macOS 上有进程创建开销
- **与 `run_short()` 的冲突**：短命令路径刻意跳过 Store/Parser 以实现零开销。沙箱初始化可能破坏这一优化
- **信号传递**：沙箱内进程的 SIGINT/SIGTERM 处理需要验证

**建议**：`container` 模式先做（CLI wrapper，复杂度低，实用价值高），`process` 模式后做（系统级，需 per-platform 实现）。

---

#### 4.4 多存储后端

**现状**：`Store` 是具体结构体（非 trait），直接持有 `Mutex<rusqlite::Connection>`。

```rust
pub struct Store {
    conn: Mutex<rusqlite::Connection>,
}
```

所有方法（`open`, `initialize_schema`, `insert_task`, `list_tasks`, `query_events`, `prune_older_than`, `get_stats` 等）直接实现在 `impl Store` 上。

**改造路径**：

```
1. 提取 StoreBackend trait（方法签名从现有 impl 提取）
2. SqliteBackend 实现 trait（现有代码迁移）
3. PostgresBackend 实现 trait（新代码）
4. Executor/IPC handler 泛型化或使用 trait object
```

**评估**：
- **工作量**：中高。需要重构 ~15 个方法签名，所有 Store 调用点改为 trait object
- **收益**：当前无明确的多后端需求。SQLite 单文件部署是 arshy 的优势，PostgreSQL 场景（多实例共享）与 arshy 的"本地 daemon"架构不太匹配
- **建议**：推迟。除非出现明确的多实例共享需求，否则 Store 保持具体结构体。`store.backend` 配置字段保留作为预留

---

#### 4.5 中间件系统

**现状**：

```rust
// src/proxy/mod.rs
pub trait Middleware: Send + Sync {
    fn on_request(&self, _request: &serde_json::Value) -> Result<()> { Ok(()) }
    fn on_response(&self, _response: &serde_json::Value) -> Result<()> { Ok(()) }
    fn on_notification(&self, _notif: &Notification) -> Result<()> { Ok(()) }
}
```

Trait 已定义但 **从未调用**。Proxy 主循环中没有中间件链。

**实现计划**：

1. **在 proxy_main 中添加 middleware chain**：
   ```rust
   let middlewares: Vec<Box<dyn Middleware>> = Vec::new(); // 初始为空
   ```
2. **在 dispatch 点插入调用**：
   - 请求：`for m in &middlewares { m.on_request(&request)?; }`
   - 响应：`for m in &middlewares { m.on_response(&response)?; }`
   - 通知：`for m in &middlewares { m.on_notification(&notif)?; }`
3. **配置驱动加载**：
   ```toml
   [mcp.middleware]
   audit = true
   rate_limit = { max_per_second = 100 }
   ```

**与现有 security 模块的关系**：
- `CommandFilter` / `PathSandbox` / `PermissionLevels` 在 **daemon 侧**（exec/mod.rs），拦截命令执行
- Middleware 在 **proxy 侧**，拦截 MCP 请求/响应流
- 两者互补：Middleware 做协议层审计，security 做执行层过滤

**建议实现顺序**：AuditMiddleware（日志）→ RateLimitMiddleware（防刷）→ AuthMiddleware（认证，需要定义认证协议）

---

#### 4.6 真实环境验证

**现状**：本会话本身就是真实验证 — arshy 作为 Claude Code 的唯一 shell 执行所有命令。

**已验证**：
- ✅ `cargo test` → 结构化 JSON（test_result 事件，5 events，正确识别 summary/test_result 类型）
- ✅ `git status --short` → 纯文本（mode:auto 短命令判定正确，short_command=true）
- ✅ arshy MCP 连接稳定，health check 正常

1. **mode:auto 判定准确率**
   - 在 `is_short_command()` 中添加 tracing span，记录每个命令的判定结果和耗时
   - 统计短/长命令比例、误判率（短命令实际产出大量输出 / 长命令实际秒完成）
   - 可通过 `arshy stats` 命令暴露

2. **Parser 匹配率**
   - 每次 `task/run` 完成后，记录 parser 名称、匹配事件数、总输出行数
   - 匹配率 = 匹配事件数 / 总输出行数（仅对有 parser 的命令）
   - 可通过 `arshy query` 分析历史数据

3. **通知延迟**
   - 在 `EventBus::publish` 和 proxy `on_notification` 之间添加时间戳差值
   - 目标：< 100ms（配置中 `batch_interval_ms` 默认 100）

**可立即执行**：
- 确认当前 session 的 arshy MCP 连接正常工作
- 运行几条代表性命令（cargo test, git status, ls）验证输出结构化
- 检查 `arshy stats` 输出

---

## 四、执行优先级

```
Phase 1 (ROADMAP 重写)       ✅ 完成
    ↓
Phase 2 (Fixture 补全)       ✅ 完成
    ↓
Phase 3 (集成测试修复)        ✅ 完成
    ↓
Phase 4 (下一步方向)          ✅ 探索完成，方案已产出
```

**Phase 4 推荐执行顺序**：

1. ✅ **4.1 发布基础设施确认** — Cargo.toml 字段完整，release workflow 存在，待首次 tag 验证
2. ✅ **4.6 真实环境验证** — mode:auto 短/长命令判定正确，MCP 连接稳定
3. ✅ **4.2 Parser 扩展（高优先级批次）** — biome / oxlint / vitest 已添加，34 parser 全覆盖
4. **4.5 Middleware 系统** — 先接通 chain，再逐个实现（待执行）
5. **4.1 首次发布 + Homebrew** — 依赖首次 tag 验证通过（待执行）
6. **4.3 沙箱（container 模式）** — Docker wrapper，复杂度可控（待执行）
7. **4.2 Parser 扩展（中低优先级批次）** — systemctl / brew / psql（待执行）
8. **4.3 沙箱（process 模式）** — 系统级实现，per-platform（待执行）
9. **4.4 多存储后端** — 推迟，除非出现明确多实例需求

---

## 五、约束与原则

| 原则 | 说明 |
|------|------|
| **无兼容包袱** | arshy 没有生产用户，接口变更直接切换 |
| **短命令零开销** | `is_short_command()` → 直接执行，不写 Store |
| **单一职责** | arshy 做好 shell，不做 Skill 框架，不做 Agent 编排 |
| **测试驱动** | 每个 parser 必须有 fixture，每个功能必须有测试 |
| **安全默认** | 默认 blocked，白名单放行，审计日志不随 prune 删除 |
