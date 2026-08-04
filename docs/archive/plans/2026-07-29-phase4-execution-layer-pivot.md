# Phase 4：执行层迁移计划（Execution Layer Pivot）

**版本:** 1.0
**日期:** 2026-07-29
**状态:** 草案，待批准
**作者:** 产品（二人）+ arshy

---

## 战略前提

本计划基于三轮战略讨论确立的判断，是 `ROADMAP-STRATEGY.md` Phase 4 的具体化：

1. **bash 在 agent 手里分层退化**——简单确定子集保留（arshy 短路径已契合）；复杂组合被 Python 脚本瓦解（LLM 主动逃离）；不可替代 shell 内核（构建/测试/运维）保留但最易错。
2. **执行成功导向的 LLM** 会自发流向成功率最高的路径，逃离 bash 不确定性，改用 Python + 库或多次简单调用。
3. **arshy 的解析价值锚点必须迁移**——从"解析 bash CLI 输出"延伸到"解析 Python 脚本失败"，并把资源压到"typed tool 与 Python 库都覆盖不了的 CLI 阵地"。
4. **护城河重铸**——从"输出漂亮"转向"可量化的修正闭环提速"，对齐执行成功导向 LLM 的训练目标。
5. **放弃输入结构化 / 意图 API**（"API 化 bash"是伪命题）。

**北极星不变**：让 AI Agent 从命令执行结果中获得比人类开发者更多的信息。载体从 shell 扩展为 bash + python 双轨。

---

## 一、开发计划

### 工作项 4.1：parser 矩阵收窄聚焦

**目标**：资源从"广度覆盖"转向"深度覆盖 LLM 无法绕过的 CLI"。

#### parser 三层分类（基于 37 个 builtin）

| 层级 | parser | 处置 | 理由 |
|------|--------|------|------|
| **深耕层**（资源倾斜） | cargo, cargo-test, npm, pnpm, go, gradle, make, jest, vitest, mocha, docker, kubectl, terraform, helm, aws, ssh, esbuild, webpack, vite, turbo, nx, pip, uv, ruff | 持续优化 fixture、context enrichment、错误码覆盖 | LLM 无法用 Python 库替代；构建/测试/运维必须落 CLI |
| **维持层**（不投入，保持现状） | tsc, clippy, eslint, prettier, oxlint, biome, git, bun, deno, swc, cc, python(现有) | 仅维护 bug 修复，不加新 pattern | LSP/Python 库替代趋势；tsc/clippy 因 cargo/TS 生态高频仍需保留基础支持 |
| **降级候选**（deprecate 评估） | curl | 标记 `deprecated = true`，文档引导用 Python requests/httpx | Python requests 已全面替代；curl parser 使用率最低 |

#### 具体动作

- `parsers/builtin/curl.toml`：添加 `deprecated = true` + `replaced_by = "python-requests"`，下个 minor 版本移除（遵循 Pattern Lifecycle deprecation cycle）
- **不新增 parser**：除非用户反馈请求特定工具（坚守"数据驱动"原则）
- 深耕层：每个 parser 补齐错误码 hint 覆盖（`parsers/errors/*.toml`），重点补 cargo/docker/kubectl 的非显而易见错误码

### 工作项 4.2：Python 执行层结构化解析（前沿下注）

**目标**：跟着 LLM "用 Python 替代复杂 bash" 的行为模式迁移价值锚点。

#### 范围（守边界）

- ✅ 做：Python 脚本执行失败的**结构化解析**——`traceback`、`AssertionError`、`unittest`/`pytest` 失败
- ❌ 不做：Python 库返回值解析（LLM 自控，不越界）；通用 Python REPL（agent 框架的职责）

#### 技术方案

1. **增强 `parsers/builtin/python.toml`**：新增 traceback pattern（stateful，多行匹配）
   - 提取字段：`file`、`line`、`exception_type`、`message`
   - 覆盖：`Traceback (most recent call last):` + `File "x.py", line N, in <func>` + `ExceptionType: msg`
2. **复用现有 context enrichment**：traceback 的 `file:line` 走 `src/daemon/context/` 现有管道，自动带 ±3 行源码 + git 改动关联——**零新架构**
3. **`arshy_exec` 接受 Python**：不新增入参，agent 自然以 `python script.py` 或 `python -c "..."` 调用，arshy 走现有短路径/长路径判定
4. **fixture 测试**：`parsers/builtin/tests/python/` 新增 traceback、AssertionError、pytest fail 三组 `.txt`/`.json` fixture

#### 接入点（已侦察确认）

- 解析层：`src/daemon/parser/toml.rs`（声明式 TOML pattern，无代码改动即可加 traceback pattern）
- 富化层：`src/daemon/context/`（现成复用）
- 短路径：`is_short_command()` 已在 `src/daemon/exec/mod.rs`，`python -c` 短脚本可纳入短路径白名单

### 工作项 4.3：修正闭环指标（arshy analyze 增强）✅ 已落地（2026-08-03）

**目标**：把护城河 reframe 落地为可量化指标，向 LLM 与用户证明"经 arshy 执行修正更快"。

#### 新增 `RepairLoopMetrics` 结构

接入 `src/daemon/analytics.rs`，与现有 SummaryMetrics/TokenEfficiency/InformationDensity 并列：

```
RepairLoopMetrics {
    fix_loops: u64,              // 检测到的"错误→修复"闭环数
    avg_retries_to_fix: f64,    // 首次错误到首个成功的平均重试次数
    avg_fix_duration_ms: u64,   // 首次错误到首个成功的平均耗时
    fastest_fix_ms: u64,        // 最快修复耗时（展示潜力）
}
```

#### 检测算法（保守，避免误判）

- 在同一 `cwd` 下，按时间序扫描任务序列
- 识别"首个 error 任务 T₀"→ 向后找首个 success 任务 Tₙ（同 cwd，n≤5，时间窗 ≤30min）
- `retries = n`，`fix_duration = Tₙ.start - T₀.start`
- 超过 5 次重试或超 30min 不算闭环（避免把无关任务算进来）

#### 输出

- `arshy analyze` pretty 输出新增"修正闭环"段
- 作为执行成功导向 LLM 的正反馈信号：闭环越快，arshy 价值越被数据证明

### 工作项 4.4：叙事微调

**目标**：叙事从"shell"迁移到"执行结果结构化"，容纳 bash + python 双载体。

#### 改动点

- `README.md`：副标题从 "AI Agent's native shell" 调为 "AI Agent's structured execution layer"（正文保留 shell 表述，但定位升级）
- `docs/explanation/design-principles.md`：新增一节"双断层框架"——明确输出层（已覆盖）与语言层（承认存在但不解决，由 LLM 自行选 Python 逃离）
- `docs/ROADMAP-STRATEGY.md`：Phase 4 指向本计划；风险表"Agent 变强不需要 arshy"升级为"行业 typed tool 收敛"，缓解措施指向本计划工作项 4.2 + 4.3
- `CLAUDE.md`：dogfooding 政策补充"Python 脚本执行亦走 arshy_exec"

---

## 二、优化简化计划（减法）

坚守"不过度工程"。借迁移之机做减法。

| # | 简化项 | 动作 | 收益 |
|---|--------|------|------|
| S1 | curl parser | deprecate（4.1 已述） | 减 1 个低价值 parser，文档负担降 |
| S2 | hint 合成残留核查 | 确认 `src/daemon/context/` 无 cause/fix 合成残留（philosophy 已声明移除） | 代码与文档一致，减认知噪声 |
| S3 | 短路径白名单优化 | `is_short_command()` 的 40+ 只读工具白名单复核，移除低频项 | 减维护面 |
| S4 | parser 数量上限 | 明确"不追广度"，37 为软上限，新 parser 必须用户反馈驱动 | 防止 parser 膨胀 |
| S5 | 重叠 linter 收敛评估 | 评估 oxlint/biome 与 eslint/ruff 的使用率，使用率为 0 的标记 deprecated | 减长尾 |
| S6 | 文档去重 | philosophy.md 与 design-principles.md 重叠的"结构化事件"论述合并 | 减读者负担 |

---

## 三、优先级与里程碑

| 优先级 | 工作项 | 里程碑 | 预估 |
|--------|--------|--------|------|
| **P0** | 4.2 Python 执行层（traceback parser + fixture） | 单 PR，可独立验证 | 中等（核心新功能） |
| **P0** | 4.3 修正闭环指标（RepairLoopMetrics） | 单 PR，依赖现有 analytics 框架 | 中等 |
| **P1** | 4.1 parser 收窄（curl deprecate + 深耕层 hint 补全） | 随 4.2 同期 | 小 |
| **P1** | S1–S3 简化项 | 随各工作项附带 | 小 |
| **P2** | 4.4 叙事微调（README/philosophy/ROADMAP） | 4.2+4.3 落地后统一改 | 小 |
| **P2** | S4–S6 简化项 | 4.4 同期 | 小 |

**里程碑**：4.2 + 4.3 落地后，arshy 可宣称"支持 Python 脚本失败结构化解析 + 修正闭环可量化"——这是对执行成功导向 LLM 的直接价值主张。

---

## 四、不做清单

- 不做输入结构化 / 意图 API（已定调，伪命题）
- 不做通用 Python REPL（agent 框架职责，越界）
- 不追 linter parser 广度（LSP/Python 库替代趋势）
- 不与 typed tool（Read/Write/Grep）竞争通用文件操作
- 不做 ML 压缩、不做跨 agent 记忆（与 ROADMAP 一致）
- 不新增 parser 除非用户反馈驱动

---

## 五、风险与缓解

| 风险 | 严重度 | 缓解 |
|------|--------|------|
| Python traceback 格式碎片化（CPython/PyPy/嵌套异常） | 中 | 先支持 CPython 标准 traceback，PyPy/嵌套后续按反馈加 |
| 修正闭环误判（无关任务被算入） | 中 | 保守算法：同 cwd + 重试≤5 + 时间窗≤30min |
| typed tool 收敛比预期快，CLI 执行层市场萎缩 | 中 | 锚定构建/测试/运维（typed tool 难覆盖），持续观察 |
| 用户反馈"Python 执行层没必要" | 低 | 4.2 是低风险下注，若反馈否定可降级为 maintain 层 |

---

## 六、下一步

1. 批准本计划
2. 先做 4.2 技术可行性评估（现有 python.toml 能否接纳 traceback stateful pattern，context 复用程度）——若可行直接进入实施
3. 4.3 同步设计 RepairLoopMetrics 检测算法细节
