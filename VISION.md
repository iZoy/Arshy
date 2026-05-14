# Arshy 产品愿景

> **AI Agent 唯一的 Shell。**

---

## 一句话

arshy 是 AI Agent 操作系统的 shell 层。

就像 Linux 有 `/bin/sh`，Windows 有 `cmd.exe`，AI Agent 有 arshy。Agent 发出的每一条 shell 命令都经过它，它把混乱的 terminal 输出变成 Agent 能理解的结构化语言。

---

## 定位

Arshy 不是 Agent 的"可选增强"，而是 Agent 执行 shell 的**唯一通道**。

**子定位：CLI+Skill 执行基座。**
未来 Agent 生态通过动态加载 Skill 学习人类 CLI 工具的用法（`gh pr create`, `docker compose up`, `kubectl apply`），每一条命令都必须经过 shell。arshy 就是这条必经之路——Skill 管"跑什么"，arshy 管"怎么跑"和"理解输出"。Skill 是别人的事，arshy 做好 shell 就够了，但正因为每条命令都经过它，arshy 自然成为整个 Skill 生态的执行基座。

```
Agent 动态加载 Skill → 学会用 CLI → 执行命令 → arshy → 结构化输出
                                ↑
                           Skill 的职责
                                    ↑
                             arshy 的全部职责
```

| 维度 | 原生 Bash tool | Arshy |
|------|---------------|-------|
| 输出 | 原始文本，Agent 需自己解析 | 结构化事件 (error/warning/file/line) |
| 错误定位 | 无，Agent 在文本中搜索 | 精确到 文件:行:列 + ±3 行源码上下文 |
| 长任务 | 阻塞等待或超时丢弃 | async + 实时通知 + 优雅终止 |
| 安全 | 无限制 | 命令过滤 + 路径沙箱 + 权限分级 + 审计 |
| 持久化 | 无 | SQLite 跨会话查询历史 |
| 短命令 | 直接执行 | 同样直接执行，零额外开销 |
| 任意 CLI | Agent 自己处理原始输出 | 通用 JSON parser + stderr 识别 + parse_hint，任意 CLI 输出均可结构化 |

---

## 三层形态

### 第一层：对 Agent — 更聪明的 shell

Agent 不需要知道 arshy 的存在。它调 `arshy_run`，和调原生 Bash tool 体验一样。但拿到的东西完全不同：

```
原生 Bash:                          arshy:
"Compiling foo v0.1.0              {
 error[E0308]: mismatched types     task_id: "abc-123",
  --> src/main.rs:42:10              events: [
   = note: expected type `u32`         { type: "compile_error",
      found type `String`                severity: "error",
158 | fn foo(x: u32) {                 file: "src/main.rs",
     |              ---                 line: 42,
     |                    ^^^           column: 10,
     |             expected `u32`       message: "mismatched types",
     |                found `String"     context: "fn foo(x: u32) {"
error: aborting due to..."          }
                                  ]
                                }
```

### 第二层：对开发者 — 透明的基础设施

装一次，写一次 `arshy install`，之后完全无感。

```bash
# 一次性安装
$ arshy install
Registered arshy as MCP server in ~/.claude/settings.json

# 之后 Claude Code 自动走 arshy，开发者不需要再管
# arshyd 在后台运行，自动启停，PID file 防重复
$ arshy status
daemon: running (pid 12345, uptime 3d 12h)
tasks: 847 total, 2 running, 845 completed
storage: 12.3 MB (auto-pruned, keep 30 days)
```

### 第三层：对企业 — 可控的执行审计平台

```bash
# 管理员配置安全策略
$ arshy config set executor.sandbox_mode process
$ arshy config set security.allowed_commands "cargo,npm,go,python,git"
$ arshy config set security.blocked_patterns "rm -rf /,curl.*|.*"
$ arshy config set security.audit_log ~/.local/share/arshy/audit.log

# 审计
$ arshy stats
period: last 7 days
commands: 3,421 total
  by tool: cargo 1,204 | npm 891 | python 523 | git 412 | other 391
  avg duration: 4.2s | p99: 47.3s
  error rate: 12.3%
  blocked: 23 commands rejected by policy

# 企业多实例
$ arshy config set store.backend postgres
$ arshy config set store.dsn "postgresql://arshy:***@db.internal/arshy"
```

---

## 核心价值

| 维度 | 有 arshy | 没 arshy |
|------|---------|---------|
| **Agent 理解力** | 结构化事件，精确到 file:line:col | 原始文本，Agent 自己猜 |
| **长任务** | async + 实时通知 + 优雅终止 | 超时丢弃 or 阻塞等死 |
| **安全性** | 命令过滤 + 路径沙箱 + 权限分级 + 审计 | 无限制，Agent 可以 rm -rf / |
| **可追溯** | SQLite 持久化，跨会话查询历史任务 | 无记录，用完即忘 |
| **可观测** | P99、失败率、按 parser 分组统计 | 无法诊断 |
| **可扩展** | middleware 链、container sandbox、多存储后端 | 一个铁板 |
| **部署** | daemon PID file、auto-prune、WAL checkpoint | 手动管理 |
| **任意 CLI** | 通用 JSON parser + stderr 识别，Skill 驱动的任意 CLI 均可结构化 | Agent 自己处理原始输出，Skill 价值打折 |

---

## 竞争定位

```
               Agent 可用性
                   ↑
                   |
        原生 Bash  |  ← arshy (当前)
        (无结构化,  |  (结构化, 安全, 可审计,
         无安全)    |   可扩展, 生产就绪)
                   |
                   |  ← arshy + CLI+Skill (基座)
                   |  (结构化 + 安全 + 任意 CLI 解析
                   |   + Skill 生态执行层 + Token 高效)
    ───────────────┼──────────────→ 企业可控性
                   |
        无         |  ← 传统 CI/CD
                   |  (安全但不是 Agent 原生)
```

**arshy 占据的位置：Agent 原生 + 企业可控 + CLI+Skill 执行基座。** 目前没有竞品同时做到这三点。

- 原生 Bash tool：Agent 能用，但不安全、不结构化，Skill 驱动的任意 CLI 无法理解输出
- CI/CD 系统：安全，但不是为 Agent 设计的（同步、阻塞、无 MCP）
- Docker/nsjail：沙箱，但不是 Agent 的 shell（太重、太慢）
- 纯 Skill 框架：教 Agent 用 CLI，但执行后没有结构化输出——arshy 是它们缺失的那一层

---

## 最终形态的 3 个关键词

1. **透明** — Agent 不需要学习新工具，`arshy_run` 就是它的 shell
2. **结构化** — 每一行 terminal 输出都被理解、分类、索引
3. **可控** — 企业能控制 Agent 能执行什么、不能执行什么、执行了什么

---

## 一句话总结

**arshy 让 AI Agent 从"会用 shell"变成"理解 shell"。**
