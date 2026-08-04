# GenericPairMerger: 诊断+位置事件合并

**日期:** 2026-06-30
**状态:** 待审批
**范围:** 信息效率提升 — P0 事件合并

---

## 背景

### 问题

arshy 的核心哲学是**让 Agent 读懂命令输出**，而不是让 Agent 少读。当前最大的效率损失来自一个结构性问题：高频工具（cargo、python、clippy）把一个错误拆成两个事件输出。

**cargo 编译错误的原始输出：**
```
error[E0308]: mismatched types
  --> src/main.rs:42:5
```

**arshy 当前产出两个独立事件：**
```json
{ "type": "diagnostic", "severity": "error", "code": "E0308", "message": "mismatched types" }
{ "type": "location", "severity": "info", "location": { "file": "src/main.rs", "line": 42, "column": 5 } }
```

Agent 需要自己关联这两个事件才能知道"哪个错误在哪个文件的哪一行"。这迫使 Agent 做额外推理，降低了信息效率。

### Stats 数据支撑

| 指标 | 当前值 | 说明 |
|------|--------|------|
| 结构化事件率 | 3.7% | 只有 location/code/context/hint 的事件算结构化 |
| 有 location 的事件 | 73 / 11,699 (0.6%) | 位置信息极度稀疏 |
| 有 code 的事件 | 313 | 错误码提取正常 |
| Context 丰富率 | 175 / 859 errors (20.4%) | 只有有 location 的 error 事件才能被 context 丰富 |

**根因分析：** cargo 的 `cargo-error` 模式提取 code + message，`cargo-location` 模式提取 file + line。它们是两个独立事件。diagnostic 事件没有 location，所以 ContextEnricher 无法为它附加源码上下文。这是 20.4% context 丰富率的根本原因。

### 受影响的 Parser

有 diagnostic + location 配对模式的 parser（8 个）：
- `cargo` — compile-error → location
- `clippy` — clippy-warning → location
- `python` — location → traceback-error
- `docker` — build-error → location
- `terraform` — apply-error → location
- `oxlint` — lint-error → location
- `helm` — template-error → location
- `git` — conflict → conflict-file

这 8 个 parser 覆盖了 dogfooding 数据中的 Top 2 命令（cargo 173 次、cargo-test 99 次）。

---

## 设计

### 目标

将 diagnostic + location 两个事件合并为一个完整事件，让 Agent 一次性获得错误的全部信息。

### 非目标

- 不修改现有 37 个 parser 的 TOML 定义
- 不解决稀疏 parser 的字段提取问题（cargo-test 等，P1 范围）
- 不修改 hint 管道（P3 范围）

### 架构定位

```
6层 Parser Pipeline
    |
    v
Deduplicator（连续相同事件折叠）
    |
    v
RustcContextMerger（吸收 rustc 上下文行）
    |
    v
★ GenericPairMerger（合并 diagnostic + location 事件对）★
    |
    v
Stderr severity upgrade
    |
    v
[存储 + EventBus]
    |
    v (完成后)
ContextEnricher（±3 行源码上下文）
    |
    v
HintDb lookup（错误码 → 修复建议）
```

**选择这个位置的理由：**
- 在 RustcContextMerger 之后：RustcContextMerger 处理上下文行（`|`、`42 | code`），不碰 `--> src/main.rs:42` location 行
- 在 Stderr severity upgrade 之前：合并发生在 severity 调整之前
- 在 ContextEnricher 之前（同一管道阶段）：合并后的 diagnostic 获得 location → ContextEnricher 可以为它附加源码上下文

### 合并后事件的变化

```
合并前（2 个事件）：
  Event 1: { type: "diagnostic", severity: "error", code: "E0308",
             message: "mismatched types" }
  Event 2: { type: "location", severity: "info",
             location: { file: "src/main.rs", line: 42, column: 5 } }

合并后（1 个事件）：
  Event 1: { type: "diagnostic", severity: "error", code: "E0308",
             message: "mismatched types",
             location: { file: "src/main.rs", line: 42, column: 5 } }
  Event 2: （已移除）
```

---

### 合并算法

#### 核心挑战

不同工具的 diagnostic 和 location 事件顺序不一致：

```
cargo/rustc:  diagnostic → location    （error 先，位置后）
python:       location → diagnostic    （位置先，error 后）
```

#### 算法：双缓冲 + 双向合并

状态变量：
- `pending_diagnostic: Option<usize>` — 缓冲的 diagnostic 事件索引
- `pending_location: Option<usize>` — 缓冲的 location 事件索引

处理每个事件时的逻辑：

| 条件 | 动作 |
|------|------|
| 当前是 location，有 pending_diagnostic | **前向合并**: location 吸收进 pending_diagnostic |
| 当前是 diagnostic，有 pending_location | **后向合并**: pending_location 吸收进当前 diagnostic |
| 当前是 location，有 pending_location | flush pending_location（连续多个 location，保留前一个） |
| 当前是 diagnostic，有 pending_diagnostic | flush pending_diagnostic（连续多个 diagnostic，保留前一个） |

#### 字段合并规则

```rust
fn merge_location_into(diag: &mut TaskEvent, loc: &TaskEvent) {
    if diag.location.is_none() {
        diag.location = loc.location.clone();
    }
    // severity: 保留 diagnostic 的（location 通常是 "info"）
    // type: 保留 diagnostic 的（"diagnostic"）
    // code/message: 保留 diagnostic 的（location 通常没有这些字段）
}
```

- **location**: 从 location 事件复制到 diagnostic 事件
- **severity**: 保留 diagnostic 的 severity（不取 location 的 "info"）
- **type**: 保留 diagnostic 的 type
- **code/message**: 保留 diagnostic 的

#### Event Type 识别

```rust
fn is_diagnostic(e: &TaskEvent) -> bool {
    matches!(e.event_type.as_str(), "diagnostic" | "crash")
}

fn is_location(e: &TaskEvent) -> bool {
    e.event_type == "location"
}
```

#### 算法验证

**cargo 模式（diagnostic → location）：**
```
1. diagnostic A (error[E0308]) → pending_diagnostic = A
2. location L (src/main.rs:42) → 前向合并！A 吸收 L
   → emit: { type:"diagnostic", code:"E0308", message:"...", location:{file:"src/main.rs",line:42} }
Result: 正确
```

**python 模式（location → diagnostic）：**
```
1. location L (foo.py:15) → pending_location = L
2. diagnostic B (SyntaxError) → 后向合并！B 吸收 L
   → emit: { type:"diagnostic", message:"SyntaxError...", location:{file:"foo.py",line:15} }
Result: 正确
```

**python 多帧 traceback：**
```
1. diagnostic A (header) → pending_diagnostic = A
2. location L1 (foo.py:15) → 前向合并！A 吸收 L1 → emit A
3. location L2 (baz.py:20) → pending_location = L2
4. diagnostic B (SomeError) → 后向合并！B 吸收 L2 → emit B
Result: A 有 L1 的位置，B（实际 error）有 L2 的位置。正确。
```

**连续两个 diagnostic + 一个 location：**
```
1. diagnostic D1 (error[E0308]) → pending_diagnostic = D1
2. diagnostic D2 (error[E0412]) → flush D1, pending_diagnostic = D2
3. location L (src/main.rs:42) → 前向合并！D2 吸收 L → emit D2
Result: D1 无 location，D2 有 location。Cargo 按顺序输出，location 对应最近的 diagnostic。
```

**已知限制：** Python 单帧 traceback（location → diagnostic，只有一个 location）会将 location 合并进 header 而非 error。影响范围小（多帧 traceback 正确，单帧 traceback 的 location 信息仍保留在 header 事件中）。

---

### 代码集成

#### 新增文件

**`src/daemon/parser/pair_merger.rs`** — 独立模块，约 80 行

```rust
/// 合并 diagnostic + location 事件对
/// 原地修改事件缓冲区，被吸收的位置事件设为 None
/// 返回合并的事件对数量
pub fn merge_diagnostic_location_pairs(events: &mut Vec<Option<TaskEvent>>) -> u64
```

#### 管道集成

在 `src/daemon/parser/mod.rs` 的 `ParserSession` 中：

```rust
// 现有流程（~line 310-320）：
deduplicator.filter(...)
rustc_context_merger.merge_events(...)

// 新增：
let pairs_merged = pair_merger::merge_diagnostic_location_pairs(&mut events);
```

在 `run_background()` 中，合并计数写入 `TaskMetrics`：

```rust
metrics.pairs_merged += pairs_merged;
```

#### 策略选择

在 `parse_line()` 内部运行（策略 A），而非外部运行（策略 B）。

理由：
- 合并后的事件直接进入 EventBus 和 Store，所有下游组件看到的都是合并后的事件
- ContextEnricher 自然受益（diagnostic 事件有 location 了）
- Fixture 更新是一次性工作，可用 `ARSHY_BLESS=1` 自动重新生成

#### 与现有组件的交互

| 组件 | 交互方式 |
|------|---------|
| RustcContextMerger | 无冲突。处理上下文行，不处理 location 行 |
| Deduplicator | 无冲突。在 pair merger 之前运行 |
| ContextEnricher | 正向影响：diagnostic 事件获得 location → 可被 context 丰富 |
| HintDb | 无变化：基于 code 字段查找，合并前后 code 不变 |
| Metrics | 新增 pairs_merged 计数器 |
| EventBus | 正向影响：推送的事件更完整 |

#### Metrics 扩展

在 `src/daemon/store/mod.rs` 的 `TaskMetrics` 中新增：

```rust
pub pairs_merged: u64,
```

在 `arshy stats` 和 `arshy analyze` 中体现。

---

### 测试策略

#### 1. 单元测试（pair_merger.rs 内部）

| 测试用例 | 输入 | 预期输出 |
|---------|------|---------|
| 前向合并 | `diagnostic + location` | 1 个合并事件 |
| 后向合并 | `location + diagnostic` | 1 个合并事件 |
| 连续 location | `diagnostic + location + location` | diagnostic 吸收第一个，第二个独立 |
| 连续 diagnostic | `diagnostic + diagnostic + location` | 第一个 flush，第二个吸收 location |
| 无配对 | `diagnostic + summary` | 不变，2 个事件 |
| 空缓冲区 | `[]` | 不变 |
| 已有 location | `diagnostic(带location) + location` | 保持原有 location，不覆盖 |

#### 2. Fixture 测试更新

受影响的 parser（8 个）：cargo, clippy, python, docker, terraform, oxlint, helm, git

操作步骤：
1. `ARSHY_BLESS=1 cargo test --bin arshyd` — 自动重新生成预期 JSON
2. 逐个检查合并后的 fixture，确认 location 正确附加到 diagnostic
3. `cargo test --bin arshyd` — 确认所有 fixture 通过

#### 3. 集成验证

```bash
# cargo 编译错误
arshy run "cargo build" --format json
# 检查：diagnostic 事件应包含 location 字段

# python 错误
arshy run "python3 -c 'import nonexistent'" --format json
# 检查：error 事件应包含 location

# stats 验证
arshy stats
# 检查：新增 pairs_merged 指标
```

#### 4. 回归保护

- 所有现有 42 个 fixture 测试必须通过（bless 后）
- `cargo clippy --all-targets -- -D warnings` 零警告
- `cargo fmt --all -- --check` 格式检查通过
- 现有 dogfooding 自检测试不受影响

---

### 预期效果

| 指标 | 当前值 | 预期改善 |
|------|--------|---------|
| 结构化事件率 | 3.7% | 提升（更多事件同时有 code + location） |
| 有 location 的 diagnostic 事件 | 0 | 从 0 开始增长（cargo/clippy/python 等的 diagnostic 事件获得 location） |
| Context 丰富率 | 20.4% | 提升（有 location 的 diagnostic 事件可被 ContextEnricher 丰富） |
| Agent 额外工具调用 | 4 次/修复 | 减少（Agent 不再需要关联 diagnostic + location 事件） |
| 事件数量 | 11,699 | 减少（配对事件合并后总数下降） |

---

### 文件变更清单

| 文件 | 变更类型 | 说明 |
|------|---------|------|
| `src/daemon/parser/pair_merger.rs` | 新增 | GenericPairMerger 实现 |
| `src/daemon/parser/mod.rs` | 修改 | 集成 pair_merger 到管道 |
| `src/daemon/store/mod.rs` | 修改 | TaskMetrics 新增 pairs_merged |
| `src/daemon/analytics.rs` | 修改 | 体现 pairs_merged 指标 |
| `parsers/builtin/tests/*/expected.json` | 更新 | 8 个 parser 的 fixture 重新生成 |
