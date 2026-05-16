# Parser 管道

## 概述

Parser 管道是 Arshy 的核心：将命令的原始文本输出转化为结构化事件。设计目标覆盖率为 TOML 80%、Stateful 15%、Crash/Raw 5%。

## 四级匹配

每行输出按优先级依次尝试，命中即停止：

```
┌─────────────────────────────────────────────────────────┐
│ Level 1: Stateful Parser                                │
│  - TOML stateful 模式（带 state_condition / transition）│
│  - Rhai 脚本（on_line 回调）                            │
├─────────────────────────────────────────────────────────┤
│ Level 2: Line Patterns (TOML)                           │
│  - 无状态逐行正则匹配                                   │
│  - 20 个内置 + 用户自定义                               │
├─────────────────────────────────────────────────────────┤
│ Level 3: Crash Parser                                   │
│  - 通用崩溃/异常检测                                    │
│  - 覆盖 Go/Python/Rust/Node/Shell                      │
├─────────────────────────────────────────────────────────┤
│ Level 4: Raw Fallback                                   │
│  - 原始文本，类型为 "log"                               │
│  - 永远兜底，确保每行都有事件                           │
└─────────────────────────────────────────────────────────┘
```

## 工具检测

命令执行前，Registry 通过 `detect` 列表匹配工具：

```
"cargo build" → 首词 "cargo" → 匹配 detect=["cargo"] → parser "cargo"
"npx tsc"     → 首词 "npx"   → 不匹配 detect=["tsc"] → 无 parser
```

匹配规则：命令首词（小写）与 detect 列表中的词做子串匹配。第一个命中的 parser（按优先级排序）获胜。

### 优先级与去重

- 内置 parser 默认优先级 50
- 用户 parser 优先级 100（同名覆盖内置）
- 同优先级时，用户源优先于内置源
- `raw` 兜底 parser 优先级 0，不参与 detect

## Parser 会话

检测到工具后，Engine 创建 `ParserSession`：

```rust
struct ParserSession {
    line_patterns: Vec<LinePattern>,    // TOML 无状态模式
    stateful: Option<StatefulParser>,   // 有状态解析器
}
```

会话在任务生命周期内存在。每个任务独立会话，无共享状态。

## Level 1: Stateful Parser

### TOML Stateful

从 `parser_type = "stateful"` 的 TOML 定义加载。

模式带状态条件和转换：

```toml
[[pattern]]
regex = '^npm ERR! (.+)$'
event_type = "diagnostic"
severity = "error"
state_transition = "has_error=true"
```

- `state_condition`：仅当状态匹配时激活
- `state_transition`：匹配后更新状态

状态机是简单的 key-value 存储，跨行持久。

### Rhai Script

从 `.rhai` 文件加载。脚本定义 `on_line(line, ctx)` 和 `on_complete(exit_code, ctx)`。

```rhai
fn on_line(line, ctx) {
    if line.contains("error") {
        ctx.emit("diagnostic", "error", line);
    }
}

fn on_complete(exit_code, ctx) {
    if exit_code != 0 {
        ctx.emit("summary", "error", "command failed");
    }
}
```

ctx API：`emit()`、`set()`、`get()`、`has()`。详见 [Rhai API 参考](../reference/parser-rhai-api.md)。

## Level 2: Line Patterns (TOML)

无状态逐行匹配。每行独立，不依赖其他行。

```toml
[[pattern]]
name = "cargo-error"
regex = '^error\[E(\d+)\]: (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { code = 1, message = 2 }
```

匹配流程：
1. 遍历所有 pattern（按定义顺序）
2. 正则匹配行文本
3. 提取捕获组 → 映射到事件字段
4. 返回第一个命中的 pattern

## Level 3: Crash Parser

通用崩溃检测，不依赖工具信息。

| 语言 | 检测模式 | 提取信息 |
|------|----------|----------|
| Go | `main.go:42 +0x1234` | 文件、行号 |
| Python | `File "app.py", line 10` | 文件、行号 |
| Python | `TypeError: ...` | 消息 |
| Rust | `thread 'main' panicked at ...` | 文件、行号、消息 |
| Node.js | `Error: ...` / `ReferenceError: ...` | 消息 |
| Node.js | `at func (file.js:42:10)` | 文件、行号 |
| Shell | `Segmentation fault` | 消息 |

事件类型统一为 `crash`，严重度为 `error`，`code` 字段标识语言。

## Level 4: Raw Fallback

所有级别均未匹配时，生成原始 log 事件：

```json
{
  "seq": 1,
  "type": "log",
  "severity": null,
  "message": "原始行文本"
}
```

## 任务完成

命令执行完毕后，调用 `on_complete(exit_code, seq)`：

- TOML parser：无操作
- Stateful parser：检查状态，生成 summary 事件
- Rhai script：调用 `on_complete(exit_code, ctx)` 回调

```rhai
fn on_complete(exit_code, ctx) {
    let count = ctx.get("error_count");
    if exit_code == 0 {
        ctx.emit("summary", "info", "succeeded");
    } else {
        ctx.emit("summary", "error", "failed with " + count + " errors");
    }
}
```

## 覆盖率分布

```
                    ┌─────────────┐
                    │  raw (5%)   │  未知工具 / 通用命令
                    ├─────────────┤
           ┌────────┤crash (5%)   │  崩溃/异常
           │        ├─────────────┤
    ┌──────┤        │stateful(15%)│  npm, webpack, 自定义脚本
    │      │        ├─────────────┤
    │      └────────┤toml (75%)   │  tsc, cargo, jest, eslint...
    │               └─────────────┘
    │
    └── 目标：70%+ TOML 覆盖主要 CI/构建工具
```

## 热重载

`parser.hot_reload = true` 时，文件系统监视器（notify v7）监听 `~/.arshy/parsers/` 目录。变更时：

1. 重新加载所有 parser（内置 + 用户）
2. 获取 Registry 写锁
3. 替换整个 Registry
4. 释放锁

已运行的任务不受影响（使用创建时的 ParserSession）。新任务使用新 Registry。
