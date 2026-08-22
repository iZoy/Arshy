# arshy vs RTK vs Headroom：定位对比

> **重要**：本文档是**架构层面**的对比，不是 head-to-head benchmark。我们没有在同等环境下跑过三方对比，所以所有数字都标注"架构判断"而非"实测数字"。

**最后更新**：2026-08-23 · 与 `docs/marketing/CLAIMS.md` 同步

---

## TL;DR

| 工具 | 解决什么问题 | 怎么做 | 与 arshy 关系 |
|---|---|---|---|
| **RTK** | 输出文本太长 | Hook 拦截 + 100+ 硬编码过滤器 | 同层（后处理），可叠加 |
| **Headroom** | 输出太长 + 跨 agent 记忆 | ML 模型压缩 + 跨 session 记忆 | 同层（后处理），可叠加 |
| **arshy** | 输出无结构 | **执行期**重组为结构化事件 | 互补层（执行层），与它们叠加 |

**关键区分**：RTK/Headroom 在命令**跑完之后**压缩文本；arshy 在命令**运行期间**重组输出。

---

## 详细对比

### RTK（runkit-cli，~61k★ on GitHub）

**RTK 解决什么**：命令输出太多文本字符，Agent 看到 200 行 cargo 输出要花很多 token 解析。

**RTK 怎么做**：
- 在 shell 的 PRE_EXEC / POST_EXEC hook 中拦截命令
- 用 100+ 硬编码的正则匹配把特定工具的输出替换成更短的格式（如 `cargo test` 输出折叠成 `5 passed, 2 failed`）
- 输出替换后的文本给 Agent

**RTK 不做什么**：
- 不解析输出结构（不抽取 `file:line`、不分类 severity）
- 不跨 session 记忆
- 不在执行期干预

**与 arshy 对比**：

| 维度 | RTK | arshy |
|---|---|---|
| 干预点 | 命令**之后**（hook） | 命令**期间**（PTY → 解析管道） |
| 输出形式 | 仍然是文本（缩短后） | 结构化事件（JSON） |
| `file:line` 提取 | ❌ 没有 | ✅ 内置 |
| 源码上下文 | ❌ 没有 | ✅ ±3 行 |
| 跨 session 记忆 | ❌ 没有 | ✅ JSONL store |
| Hook 配置复杂度 | 中（要装 shell hook） | 低（透明） |
| 性能开销 | ~1ms（正则替换） | ~10-50ms（解析管道） |
| 覆盖工具数 | 100+ | 37 |
| 跨生态 e2e 测试 | 不详 | 5 个跨生态真实命令 |

**可叠加**：RTK 拦截 raw output 后，如果想再过 arshy 解析管道，理论可行（没测）。

---

### Headroom（~22k★）

**Headroom 解决什么**：上下文窗口爆炸；多 agent 共享上下文。

**Headroom 怎么做**：
- 用 ML 模型把长输出压缩为短摘要
- 维护跨 agent 共享的"记忆层"（key facts 跨 session 保留）
- 上下文窗口要爆时自动触发压缩

**Headroom 不做什么**：
- 不解析结构（也是文本压缩）
- 不在执行期干预
- 需要 LLM 调用（ML 推理有成本）

**与 arshy 对比**：

| 维度 | Headroom | arshy |
|---|---|---|
| 干预点 | 上下文窗口**内** | 命令**执行期** |
| 输出形式 | 压缩后的文本 | 结构化事件 |
| LLM 调用 | 是（每次压缩） | 否 |
| 跨 session 记忆 | ✅ key facts | ✅ 完整 task 历史 |
| Token 节省 | 中（取决于 ML 模型） | 高（结构化本来就更短） |
| 部署成本 | 高（需 ML 服务） | 低（纯 Rust 二进制） |

**可叠加**：Headroom 在 Agent 循环里把 arshy 的结构化响应再压一遍——但通常没必要（结构化已经够短）。

---

## arshy 独有的能力

这些是 RTK 和 Headroom **都没有**的：

### 1. 真正的结构化（不是文本压缩）

```jsonc
// arshy 的 run 响应
{
  "task_id": "...",
  "status": "failed",
  "exit_code": 1,
  "events": [
    {
      "type": "diagnostic",
      "severity": "error",
      "code": "E0308",
      "message": "mismatched types",
      "location": {"file": "src/main.rs", "line": 15, "column": 5},
      "context": {
        "before": ["fn main() {"],
        "line": "    let x: i32 = \"string\";",
        "after": ["    println!(\"{}\", x);"]
      }
    }
  ]
}
```

vs RTK/Headroom 的输出**仍然是字符串**——Agent 还是要 parse。

### 2. 跨 session 查询

```bash
$ arshy query --code E0308
# 找到历史上所有 E0308 类型错误，按时间倒序，附带源文件位置
```

### 3. 错误码参考表（按需返回）

```bash
$ arshy query "kubectl exit 137" 
# 返回: "137 = OOMKilled by kubelet"
```

### 4. 测量诚实性（savings_basis）

RTK/Headroom 节省的 token 是估算的。arshy 有 `savings_basis: measured | estimated | none` 字段明示数字基础。

### 5. 沙箱 + 审计

默认 passive 审计（`workspace` 模式可强制沙箱）。RTK/Headroom 不管这个层。

---

## 何时选哪个？

| 你的痛点 | 选 | 为什么 |
|---|---|---|
| Agent 输出太长费 token，但你能接受文本格式 | **RTK** | 成熟、覆盖广、生态强 |
| 上下文窗口频繁爆炸 | **Headroom** | ML 压缩 + 跨 session 记忆 |
| Agent 拿不到 `file:line` / 错误码 / 源码上下文 | **arshy** | 结构化事件省去 Agent 自己 parse |
| Agent 在不同 session 重复犯同样错误 | **arshy** | 跨 session `query` 复用历史 |
| 你用 docker/kubectl 想知道非显而易见退出码 | **arshy** | 内置错误码参考表 |
| 想让 Agent 走 cargo / npm / go 测试时立即知道失败点 | **arshy** | 6 层解析管道专门做这个 |

**现实答案**：很多人会**叠加**——RTK 做通用过滤，arshy 做结构化增强。

---

## 为什么我们没跑 head-to-head benchmark

诚实原因：

1. **环境差异大**：RTK 装 shell hook，Headroom 调 ML 服务，arshy 起 daemon——同一台机器跑三个东西互相干扰
2. **生态覆盖不一致**：RTK 100+ 工具，Headroom 通用的，arshy 37 parser——选哪 37 跑？跑什么场景？
3. **输入数据敏感**：命令输出因 repo 而异，三方在同一个 repo 上的数字才公平
4. **诚实性问题**：三方各有营销数字，谁的 baseline 都不一样——容易变成 RPS 游戏

**如果你想跑 head-to-head**：[GitHub Discussions](https://github.com/iZoy/Arshy/discussions) 开帖，我们提供 arshy 端的复现脚本和基线。

---

## 性能数字的真实来源

所有 arshy 数字来自 [`scripts/measure-savings.sh`](../../scripts/measure-savings.sh) + [`docs/evidence/`](../../evidence/)。

```bash
$ ./scripts/measure-savings.sh
ECOSYSTEM  | STATUS | EVENTS | RAW(B) | DELIV(B) | SAVINGS
python     | failed | 2      | 293   | 99       | 66.2%
go         | failed | 2      | 158   | 73       | 53.8%
rustc      | failed | 2      | 333   | 67       | 79.9%
node       | failed | 8      | 770   | 162      | 79.0%
npm        | failed | 1      | 268   | 134      | 50.0%
cargo      | failed | 1      | 46    | 25       | 45.7%
```

**最低 +45.7%，最高 +79.9%，聚合 +70.0%**（measured，0 fallback）。
