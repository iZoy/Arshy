# Parser 管道

## 概述

Parser 管道是 Arshy 的核心：将命令的原始文本输出转化为结构化事件。

## 六级匹配

每行输出按优先级依次尝试，命中即停止：

```
行 → 格式检测(JSON/NDJSON/YAML/CSV) → Stateful(Rhai) → TOML(regex) → Crash(通用) → Heuristic(启发式) → Raw
```

### 1. 格式检测（新增）

自动识别结构化输出格式，跳过 regex 管道：

| 格式 | 检测方式 | 典型命令 |
|------|----------|----------|
| JSON | 整体输出是合法 JSON | `kubectl get pods -o json` |
| NDJSON | 逐行 JSON 对象（≥75% 行匹配） | `docker logs --format json` |
| YAML | `---` 开头或 `key: value` 模式 | `kubectl get pods -o yaml` |
| CSV/TSV | 一致的分隔符模式 | `docker ps`、`aws s3 ls` |

实现：`json.rs` 中的 `try_parse_line()` 作为第一级，`try_parse()` 作为全量输出检测。

### 2. Stateful Parser（Rhai 引擎）

跨行有状态匹配。适用场景：
- npm install 的多行错误块
- webpack 的 chunk 编译错误（ERROR + 后续位置行）
- 需要在多行间追踪状态的复杂格式

实现：`StatefulPattern` 含 `state_condition`/`state_transition`，跨行保持状态。Rhai 脚本模式支持 `on_line()` / `on_complete()` 回调。

### 3. Line Patterns（TOML 无状态）

逐行正则匹配。覆盖 80% 场景。37 个 builtin parser。

实现：`TomlParser::parse_line()` 按顺序尝试 pattern，第一个匹配的捕获组提取 file/line/column/code/message/severity。所有正则经过 ReDoS 静态校验。

### 4. Crash Parser

通用崩溃/堆栈检测。**不依赖 parser 选择**，始终生效。覆盖 5 种语言：

| 语言 | 检测模式 |
|------|---------|
| Go | `main.go:42 +0x1234` goroutine trace |
| Python | `File "app.py", line 42` + 异常类型 |
| Rust | `thread 'main' panicked at 'msg', src/main.rs:42:5` |
| Node.js | `Error: ...` + `at func (file.js:42:10)` |
| Shell | `Segmentation fault` / `Bus error` / `Killed` |

### 5. Heuristic Error Filter（启发式错误过滤）

在所有工具特定 parser 均未命中时，捕获行中的 error/warning 关键词：

| 关键词 | 说明 |
|--------|------|
| error, fatal error, FAILED, panic | 高严重度 |
| traceback, segmentation fault | 运行时崩溃 |
| warning, deprecated | 低严重度 |

行为：
- 从编译器风格输出中提取 `file:line:col`（如 `src/main.rs:42:10: error`）
- 设置 `type=diagnostic`（区别于 raw fallback 的 `type=log`）
- 不设置 `code` 字段（无工具特定错误码）

实现：`heuristic.rs` 中的 `try_parse_heuristic()`。

### 6. Raw Fallback

所有未匹配行归类为 `log` 事件。`classify_severity()` 通过关键词（error/fatal/failed/panic/warning/deprecated）自动判断严重度。stderr 行额外经过 `stderr_looks_like_error()` 修正。

## 智能输出

命令完成后，自动计算：

| 字段 | 说明 | 用途 |
|------|------|------|
| **summary** | 按 type/severity 分组统计 | Agent 直接读 `summary.by_severity.error == 0` |
| **root_cause** | 第一个 error 级别事件 | Agent 直接读 `root_cause.message` |
| **project_context** | 失败时自动附加 git diff + 错误-变更关联 | Agent 看 `project_context.correlated_errors` 判断哪些错误由最近改动引起 |

## 事件去重

连续相同事件（相同 event_type + message）自动折叠为单条事件，后缀 "(repeated N times)"。
典型场景：cargo build 产生的 50 条相同 warning → 折叠为 1 条 "unused variable (repeated 50 times)"。

实现：`Deduplicator` 在 parser 输出和 store 插入之间运行。

## 上下文丰富

错误/警告事件自动附带源码上下文（前后 3 行代码）：

```json
{
  "type": "diagnostic", "severity": "error",
  "location": {"file": "src/main.rs", "line": 42},
  "context": {
    "before": ["fn main() {", "    let x = 1;", "    let y = 2;"],
    "line": "    let z = x + y + \"hello\";",
    "after": ["    println!(\"{}\", z);", "}", ""]
  }
}
```

Git 关联：失败命令自动标记哪些错误文件在最近提交中被修改。
```json
"project_context": {
  "git_diff_stat": " src/main.rs | 3 ++-",
  "changed_files": ["src/main.rs"],
  "correlated_errors": [{"file": "src/main.rs", "recently_changed": true}]
}
```

实现：`ContextEnricher`（文件缓存）+ `GitCorrelation`（git diff --name-only）

## 错误码修复建议

错误事件自动附带修复建议（基于 43 个常见错误码）：

```json
{
  "type": "diagnostic",
  "severity": "error",
  "code": "E0308",
  "message": "mismatched types",
  "hint": {
    "cause": "Type mismatch — expected one type but found another",
    "fix": "Check expected type, convert with .into(), as, or From trait"
  }
}
```

支持的语言和错误码数量：

| 语言 | 工具 | 错误码数 |
|------|------|---------|
| Rust | cargo/rustc/clippy | 3 |
| TypeScript | tsc/eslint/biome | 15 |
| Python | python/pytest/ruff | 14 |
| Go | go | 11 |

实现：`HintDb` 从 `parsers/errors/*.toml` 加载，编译时通过 `include_str!` 嵌入二进制。

## 工具检测

`ParserRegistry::detect()` 按优先级顺序匹配：
1. `detect_full` → 完整命令 starts_with（如 `"cargo test"`）
2. `detect` → 首词 starts_with（如 `"cargo"`）
3. 链式命令（`&&`/`||`/`;`）→ 分段重试每个 segment 的首词

检测到工具后，`probe_version()` 缓存工具版本（SQLite，24h TTL），版本约束（min_version/max_version）自动过滤不兼容 parser。

## Parser 会话生命周期

```
创建 ParserSession
  ├─ 获取 registry entry
  ├─ 检查弃用 pattern（有 replaced_by 则跳过，否则 warn）
  ├─ 构建 TomlParser（一次性 clone patterns）
  └─ 构建 StatefulParser（如适用）
       │
逐行 feed ──→ parse_line(line, seq)
       │         ├─ 格式检测（JSON line）
       │         ├─ StatefulParser::feed_line()
       │         ├─ TomlParser::parse_line()
       │         ├─ crash::try_parse_crash()
       │         ├─ heuristic::try_parse_heuristic()  ← 新增
       │         └─ raw_event()
       │
       ├─ Deduplicator::feed()  ← 新增
       │
完成 ──→ ContextEnricher::enrich()  ← 新增
         ├─ HintDb 查表附加 hint    ← 新增
         ├─ StatefulParser::on_complete()
         ├─ compute_enhanced_project_context()
         ├─ compute_summary()
         └─ extract_root_cause()
```

## 测试体系

每个 parser 有 fixture 测试（`parsers/builtin/tests/<name>/`）。`ARSHY_BLESS=1` 模式自动生成 expected JSON。

```bash
# 运行全部 parser 测试
cargo test --bin arshyd fixture

# 更新 fixture（修改 parser 后）
ARSHY_BLESS=1 cargo test --bin arshyd fixture
```

分字段精度：type/severity/code/file/line 各字段独立追踪匹配率，fixture 测试要求 ≥95%。
