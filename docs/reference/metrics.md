# arshy 指标定义（Metrics Glossary）

本文件定义 `daemon/stats`、`daemon/analyze` 与 `arshy benchmark` 三个端点暴露的所有指标。
刻意写得啰嗦——任何带数字的指标都必须可被外部独立验证。

> **诚实原则**：arshy 在内部统计数字上追求精确，对外不展示未经证实的性能声明。
> 任何带 % 的指标在代码中都有对应的 `*_basis` / `*_fallback_*` 字段标明测量基础。

---

## 1. 任务计数（task counts）

| 字段 | 含义 |
|---|---|
| `total_tasks` | 守护进程生命周期内累计的所有 task 数（含运行中/已完成/失败/被杀/超时） |
| `by_status.{running,completed,failed,killed,timeout}` | 按终态分桶 |
| `purpose_breakdown[]` | 按 `purpose` 字段分桶（`real` / `dogfood` / `sample`），把测试负载与真实开发负载分开 |
| `failure_rate` | `(failed + timeout) / total * 100` |

**陷阱**：dogfood 任务的失败率会显著拉高整体失败率——这是 arshy 自己开发的真实信号，
不是用户场景。读这个数字时请用 `purpose_breakdown` 拆开看。

---

## 2. 事件统计（event stats）

| 字段 | 含义 |
|---|---|
| `total_events` | 持久化的事件总数（含 diagnostic / log / summary / location / test_result） |
| `total_errors` | `severity == "error"` 的事件数 |
| `dedup_collapsed` | 被去重器折叠的连续重复行数 |
| `correlated_errors` | 与近期 git 变更关联的错误事件数 |
| `parser_coverage_pct` | 非 log 事件 / 全部事件 × 100 |

**诚实边界**：`parser_coverage_pct` 只统计"事件是否被 parser 处理"，**不**等同于"输出被结构化的比例"——
短命令走零开销路径，不产生事件，因此这个指标对短命令不可见。

---

## 3. Token 节省指标（marketing-only，**不展示给用户**）

> ⚠️ 这一节描述的指标是**项目宣传用的**，**默认不在 CLI pretty 输出中展示**。
> 任何想引用 "arshy 节省 X% token" 的场景必须同时引用 `savings_basis` 字段。

### 字段

| 字段 | 含义 |
|---|---|
| `total_raw_output_bytes` | 所有 task 的 PTY 原始输出字节数 |
| `total_agent_delivered_bytes` | 实际送给 agent 的字节数（结构化事件 JSON 序列化） |
| `estimated_token_savings_pct` | `(raw - delivered) / raw * 100` |
| `noise_pct` | `skipped / (skipped + visible) * 100`，skipped = log 类事件 |
| **`savings_basis`** | 测量基础：`"measured"` / `"estimated"` / `"none"` |
| **`savings_fallback_task_count`** | 走了 10% 兜底的任务数（`None` = 0） |

### 三种 `savings_basis` 的区别

| 值 | 含义 | 可信度 |
|---|---|---|
| `"measured"` | 每个 task 都有精确记录的 `agent_delivered_bytes` | **高**——可以引用 |
| `"estimated"` | 至少一个 task 走了 10% 兜底（task 有事件但没经过 enrichment 路径） | **中**——需注明估算 |
| `"none"` | 还没有任何 raw output（daemon 刚启动 / 还没跑任务） | **N/A** |

### 10% 兜底是什么？为什么要诚实标记？

`agent_delivered_bytes` 在 enrichment 阶段由 `calculate_agent_delivered_bytes()` 精确计算（基础行 + root cause + git diff stat）。
但**只有走完 enrichment 的 task 才有这个数**——daemon 在某些路径下不调用 enrichment
（例如某些短命令变体、错误分类路径、legacy code path）。

对这种 task，`get_stats()` 用 `raw / 10` 兜底（即假设结构化输出约是 raw 的 10%）。
这会**人为放大**节省率：从真实 70% 跳到 91%。

**修复路径**：`savings_basis="estimated"` 时不应被当作实测值引用——必须搭配样本量与
`scripts/measure-savings.sh` 的复现结果。

### 获取方式

```bash
# JSON（包含完整字段）
arshy stats --format json

# analyze（包含 token_efficiency 块）
arshy analyze --format json

# 一键复现脚本（v0.3.0+）
./scripts/measure-savings.sh
```

---

## 4. 修复闭环指标（repair loop）

| 字段 | 含义 |
|---|---|
| `fix_loops` | 检测到的 "error → success" 闭环数（同 cwd，real purpose） |
| `avg_retries_to_fix` | 每个闭环平均尝试次数 |
| `avg_fix_duration_ms` | 从第一次失败到第一次成功的平均 wall-clock 时间 |
| `fastest_fix_ms` | 历史最快修复时间 |

**前置条件**：必须有 ≥2 个 task 在同 cwd 上一失败一成功才能形成一个 fix_loop。
新启动的 daemon 或单一工作流通常显示 `fix_loops: 0`——这是正常的，不代表产品失败。

---

## 5. 载体分布（Q1 telemetry，ADR-0006）

`command_patterns.carrier_distribution` 字段：

| carrier | 含义 |
|---|---|
| `shell` | 直接 CLI 调用（`cargo test`、`docker ps`） |
| `shell_composite` | bash 组合（`cd x && make`、管道 `cmd1 | cmd2`） |
| `python` | `python` / `python3` 解释器调用 |
| `script_other` | 其他脚本解释器（`ruby`、`node -e`） |
| `unknown` | 未能分类 |

**重要**：`typed tool`（agent 通过结构化工具直接调用能力）**永远不经过 arshy**，
因此本指标 100% 不可观测。这是 arshy 的存在边界，不是 bug。

---

## 6. Parser benchmark 指标（`arshy benchmark`）

| 字段 | 含义 |
|---|---|
| `total_fixtures` | parser fixture 数量（当前 49 组） |
| `compression_ratio` | raw text bytes / structured JSON bytes（越大表示压缩越多） |
| `error_speed_advantage_pct` | 结构化事件定位错误的速度优势 |
| `avg_accuracy` | fixture 字段匹配率（target ≥ 95%） |

**与 token 节省指标不同**：benchmark 完全基于**固定 fixture**（不退化、可复现），
所以可以安全引用，且 `render_benchmark` 仍然展示 TOKEN EFFICIENCY 块。

---

## 引用守则

1. **不要引用 `estimated_token_savings_pct`** 当作"实测性能"——除非 `savings_basis="measured"`；
2. **不要跨生态外推**——dogfooding 数据 90% 是 Rust，引用前请用 `measure-savings.sh` 复现你的目标生态；
3. **不要在没有 ≥100 个真实 task 的样本上声称指标稳定**——< 100 时所有百分比都偏噪声大；
4. **可以引用的指标**：`parser_coverage_pct`（基于固定事件分类）、`compression_ratio`（基于固定 fixture）、`dedup_collapsed`（精确计数）。
