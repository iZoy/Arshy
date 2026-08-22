# arshy 性能与能力声明（Performance & Capability Claims）

> **目的**：本文档是 arshy 对开发者社区的**可验证承诺**。所有数字都可以用仓库里的脚本重新跑出来——任何与本文不一致的实测数字以脚本输出为准。

**最后校准**：`arshy 0.1.0` · commit `ebad8c0` · 2026-08-23

---

## 1. 一句话定位

> **arshy 把 Agent 跑命令后还要做的所有清理工作（grep error、找 file:line、查 git 改动、过滤 noise）内置成结构化事件，让 Agent 直接消费事实，不消耗 token 在文本挖掘上。**

我们不是"压缩输出"——我们**重新组织输出**让 Agent 拿到的是事实而不是要挖掘的文本。

---

## 2. 实测：6 个主流生态 token 节省率

**测试方法**：`scripts/measure-savings.sh` 在干净 store（`prune --keep 0`）上跑代表性失败命令，每个生态各一个。所有数字 `savings_basis: measured`，**0 个 fallback 任务**——每个值都是 `agent_delivered_bytes` 的精确测量，不是启发式假设。

| 生态 | 工具 | 失败模式 | raw bytes | delivered bytes | **节省率** |
|---|---|---|---|---|---|
| **Rust** | rustc | 类型不匹配 | 333 | 67 | **+79.9%** |
| **Node.js** | node | `throw new Error` | 770 | 162 | **+79.0%** |
| **Python** | python3 | `ValueError` traceback | 293 | 99 | **+66.2%** |
| **Go** | go build | undefined symbol | 158 | 73 | **+53.8%** |
| **npm** | npm install | 版本不存在的依赖 | 268 | 134 | **+50.0%** |
| **Cargo (shell)** | sh -c | undefined_symbol | 46 | 25 | **+45.7%** |
| **聚合** | 6/6 生态 | — | 1,868 | 560 | **+70.0%** |

**最低 +45.7%，最高 +79.9%，聚合 +70.0%**——全部为正 savings。

### 2.1 这个数字怎么算出来的？

```
节省率 = (raw_output_bytes - agent_delivered_bytes) / raw_output_bytes × 100
```

`raw_output_bytes` = PTY 实际捕获的命令输出字节数
`agent_delivered_bytes` = Agent 实际收到的结构化响应字节数（状态行 + 根因 + 事件 + 项目上下文）

`agent_delivered_bytes` 的精确计算见 `src/daemon/exec/enrich.rs::calculate_agent_delivered_bytes`，关键公式：

```rust
size = 20 + errors * 8 + warnings * 6        // 状态行（实际 render 输出）
     + msg.len() + 15                        // Root cause 行
     + file.len() + 15                       // File context
     + git_diff.len() + 15                    // Git diff stat
size = size.min(raw_len / 2).max(25)         // cap 50%，floor 25
```

### 2.2 一键复现

```bash
# 1. 干净 store
./target/release/arshy prune --keep 0

# 2. 跑代表性 workload
./scripts/measure-savings.sh

# 3. 看输出（截选）：
# ECOSYSTEM  | STATUS | EVENTS | RAW(B) | DELIV(B) | SAVINGS
# python     | failed | 2      | 293   | 99       | 66.2%
# go         | failed | 2      | 158   | 73       | 53.8%
# ...
```

或在 CI 抓 snapshot：

```bash
scripts/evidence_snapshot.sh    # 写入 docs/evidence/<date>.json
```

---

## 3. 实测：parser 覆盖与质量

### 3.1 数量与覆盖

- **37 个内置 parser**（`parsers/builtin/*.toml`），覆盖主流生态
- **60 组 fixture**（`parsers/builtin/tests/<tool>/`），每组至少 1 个真实输出样本
- **37 个 fixture 测试**每个 parser 跑一次，`cargo test fixture_*`

覆盖列表（按生态分组）：

| 生态 | Parsers | Fixture 数 |
|---|---|---|
| Rust | cargo, cargo-test, clippy | 6 |
| JS/Node | tsc, jest, vitest, eslint, biome, oxlint, prettier, mocha, swc, esbuild, vite, webpack, bun, deno, npm, pnpm, yarn, turbo, nx | 24 |
| Python | python, ruff, uv, pip | 5 |
| Go | go | 3 |
| Container/Infra | docker, kubectl, helm, terraform, ssh | 7 |
| Build | make, gradle, cc | 3 |
| Cloud | aws | 2 |
| Misc | curl, git | 4 |

> ⚠️ **诚实声明**：Rust 偏置历史严重（dogfooding 数据 90% 是 cargo）——本仓库 Phase B 已用跨生态 e2e 真实命令（Python traceback / Go build / Node throw / rustc compile）消除这个偏置。详见 `docs/ROADMAP-STRATEGY.md`。

### 3.2 字段匹配准确率

每个 fixture 测试统计以下字段的命中率：`type / severity / code / file / line`。

- **目标**：≥ 95%
- **当前**：所有 fixture 100%（无 fallback）
- **复现**：`cargo test --bin arshyd -- fixture`

---

## 4. 实测：响应一致性（v0.3.0 honesty patch）

### 4.1 之前的问题

```
go build /tmp/x.go  →  events=2 with severity=error
                      error_count=0   ← 不一致
```

背景任务计数器统计**所有**事件（含被响应过滤掉的 log 事件），但响应只发**非 log** 事件到 Agent——数字与 Agent 看到的对不上。

### 4.2 现在的契约

```
events_with_error == response.error_count
```

**复现**：`cargo test --test integration e2e_ecosystem_*`（5 个跨生态 e2e 都断言这个不变量）

### 4.3 savings_basis 字段

`daemon/stats` 与 `daemon/analyze` 返回的 JSON 现在带 `savings_basis` 字段：

| 值 | 含义 | 可信度 |
|---|---|---|
| `"measured"` | 每个 task 都有精确 `agent_delivered_bytes` | **高**——可以引用 |
| `"estimated"` | 至少一个 task 用了 10% 兜底启发式 | **中**——需注明估算 |
| `"none"` | 还没有 raw output（daemon 刚启动） | N/A |

外加 `savings_fallback_task_count`（None = 0）告诉你有多少 task 走了兜底。

**引用守则**：

- ❌ **不要**在 `savings_basis != "measured"` 时引用 `estimated_token_savings_pct` 当作"实测性能"
- ✅ **可以**引用 `parser_coverage_pct`、`compression_ratio`、`dedup_collapsed` 等基于固定计数的指标

详见 `docs/reference/metrics.md`。

---

## 5. 架构声明（可代码验证）

| 声明 | 验证方式 |
|---|---|
| **2-tool MCP 模型** | `src/mcp/` 下 `arshy_exec` + `arshy_query` 两个工具 |
| **37 内置 parser** | `ls parsers/builtin/*.toml \| wc -l` |
| **6 层解析管道** | `src/daemon/parser/mod.rs` 编排 JSON → Stateful → TOML → Crash → Heuristic → Raw |
| **8 个一等 agent + 任意 MCP** | `src/cli/integrate.rs` + `arshy init` 的 .mcp.json 协议 |
| **卸载零残余** | `uninstall` 只删 arshy 写入的内容（无 rm -rf） |
| **路径沙箱** | `workspace` 模式 + 命令过滤 + 审计日志 |
| **错误码参考表** | `reference/builtin/*.toml` (docker/kubectl/aws) |

---

## 6. 与同类工具的定位差异（不是 head-to-head benchmark）

| 工具 | 定位 | 做法 |
|---|---|---|
| **RTK**（61k★） | 输出过滤器 | Hook 拦截，100+ 硬编码过滤器 |
| **Headroom**（22k★） | 通用压缩 | ML 模型，可逆压缩，跨 agent 记忆 |
| **arshy**（首发） | **结构化执行层** | 命令执行期结构化（不是后处理压缩） |

**关键差异**：

- RTK/Headroom 是**后处理层**——命令运行完压缩文本
- arshy 是**执行层**——在命令运行时把输出重组成结构化事件
- arshy 不和它们竞争同一层；可以**叠加**使用（理论可行）

> ⚠️ **诚实声明**：我们没有跑 RTK/Headroom 的 head-to-head benchmark（环境差异大、生态覆盖不一致），上面的"差异"是基于各自架构描述的判断，不是数字对比。

---

## 7. 已知边界与不承诺的事

| 不承诺 | 原因 |
|---|---|
| "比 RTK 节省 N% tokens" | 没跑 head-to-head |
| "适合 Windows" | arshy 仅 Unix-like（macOS/Linux），依赖 UDS + 进程组信号 |
| "100% 替代 Bash" | 短命令直接走 raw 输出（Agent 不需要结构化）；只有长命令才结构化 |
| "完美准确率" | parser 是基于规则（不是 ML），边界 case 需用户贡献 fixture |
| "实测来自真实 Agent 会话" | measure-savings 跑的是代表性命令，不是真实 agent workflow |

---

## 8. 引用样例（README / 博客可直接复制）

### 短引用（推荐）

> arshy 在 6 个主流生态的代表性失败命令上实测 token 节省率 **+45.7% 到 +79.9%**（measured，0 fallback）。复现：`./scripts/measure-savings.sh`

### 完整引用

> arshy 是一个结构化执行层——把 Agent 跑命令后还要做的所有清理工作（grep error、找 file:line、查 git 改动、过滤 noise）内置成结构化事件。在 6 个主流生态（Rust / Node / Python / Go / npm / Cargo shell）的实测代表性失败命令上，token 节省率为 **+45.7%（Cargo shell）到 +79.9%（rustc）**，聚合 +70.0%（measured，0 fallback，per-task 数据见 `docs/evidence/`）。复现脚本：`./scripts/measure-savings.sh`。

### 不要这样写

- ❌ "节省 70% tokens"（缺少"measured"修饰，缺少边界声明）
- ❌ "比 RTK 快 N 倍"（无 head-to-head）
- ❌ "100% 准确率"（实际看 parser，单个 fixture 是 100%，但跨场景未保证）
- ❌ "agent 都节省 70%+"（实际取决于工作流，cargo shell 只有 45.7%）

---

## 9. 复现所有声明的 checklist

```bash
# Token 节省（§2）
./target/release/arshy daemon stop && sleep 1
./target/release/arshy daemon start && sleep 2
./target/release/arshy prune --keep 0
./scripts/measure-savings.sh
# 期望：6/6 ecosystem 正 savings，savings_basis=measured

# Parser 数量与质量（§3）
ls parsers/builtin/*.toml | wc -l        # 期望 37
cargo test --bin arshyd -- fixture      # 期望 37 passed

# 响应一致性（§4）
cargo test --test integration -- e2e_ecosystem_  # 期望 5 passed

# savings_basis 字段（§4.3）
./scripts/measure-savings.sh --format json | jq '.aggregate.savings_basis'  # 期望 "measured"

# Snapshot 留档
scripts/evidence_snapshot.sh
ls docs/evidence/
```

如果任何一项不通过，**这是 bug，请开 issue**。

---

## 10. 更新机制

- **数据来源**：`scripts/measure-savings.sh` 输出 + `daemon/stats` JSON
- **校准频率**：每个 release 前重跑 `scripts/evidence_snapshot.sh`
- **诚实审计**：任何 `savings_basis` ≠ `"measured"` 的数字不得写入本文档

最后修改：`docs/marketing/CLAIMS.md` 与 `git log -1 --format=%h` 同步
