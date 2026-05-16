# TOML Parser 格式参考

## 文件位置

| 来源 | 路径 | 说明 |
|------|------|------|
| 内置 | 编译时嵌入 | 20 个 builtin parser |
| 用户 | `~/.arshy/parsers/*.toml` | 自定义 parser |

同名用户 parser 覆盖内置 parser。

## 结构

```toml
[meta]
name = "tsc"
description = "TypeScript compiler"
detect = ["tsc"]
parser_type = "toml"        # "toml"（默认）或 "stateful"
priority = 50
min_version = "4.0.0"       # 可选
max_version = "6.0.0"       # 可选

[[pattern]]
name = "ts-error"
regex = '^(.+?)\((\d+),(\d+)\): error TS(\d+): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, column = 3, code = 4, message = 5 }
```

## [meta] 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | ✅ | parser 唯一名称 |
| `description` | string | | 描述 |
| `detect` | string[] | ✅ | 命令匹配词列表 |
| `parser_type` | string | | `"toml"`（默认）或 `"stateful"` |
| `priority` | integer | | 优先级，默认 50，高值优先匹配 |
| `min_version` | string | | 最低工具版本（语义化） |
| `max_version` | string | | 最高工具版本（语义化） |

### detect 匹配规则

命令首词（小写）与 `detect` 列表中的词（小写）做子串匹配：

- `detect = ["cargo"]` → 匹配 `cargo build`、`cargo test`
- `detect = ["tsc"]` → 匹配 `npx tsc --noEmit`（首词 `npx` 不匹配，但 `tsc` 是子串则不匹配 — 需要首词包含 detect 词）

> **注意**：匹配的是命令首词。`npx tsc` 的首词是 `npx`，不会匹配 `detect = ["tsc"]`。

## [[pattern]] 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | ✅ | 模式名称（用于日志） |
| `regex` | string | ✅ | 正则表达式（Rust regex 语法） |
| `event_type` | string | ✅ | 事件类型 |
| `severity` | string | ✅ | 严重度：`error` / `warning` / `info` |
| `fields` | map | | 捕获组映射 |
| `state_condition` | string | stateful 时 | 状态条件 `"key=value"` |
| `state_transition` | string | stateful 时 | 状态转换 `"key=value"` |

### fields 映射

将正则捕获组映射到事件字段：

| key | 映射到 | 说明 |
|-----|--------|------|
| `file` | `location.file` | 文件路径 |
| `line` | `location.line` | 行号 |
| `column` | `location.column` | 列号 |
| `code` | `event.code` | 错误码 |
| `message` | `event.message` | 消息文本 |
| `severity` | `event.severity` | 覆盖静态 severity |

值为正则捕获组序号（从 1 开始）。

### event_type 常用值

| 值 | 说明 |
|----|------|
| `diagnostic` | 编译/lint 诊断 |
| `location` | 文件位置信息 |
| `test_result` | 测试结果 |
| `summary` | 汇总信息 |
| `log` | 通用日志 |

## 正则表达式

使用 Rust `regex` crate（Thompson NFA 引擎）：

- **不支持**：反向引用 `\1`、前瞻/后顾断言 `(?=)`、`(?!))`
- **支持**：捕获组 `()`、字符类 `[]`、量词 `*+?{n,m}`、锚点 `^$`
- **行为**：`regex.captures(line)` 在行内搜索匹配（类似 Python 的 `search()`）
- **多行**：每次匹配一行，不含换行符

### 常用模式

```toml
# file:line:col 格式
regex = '^(.+?):(\d+):(\d+)$'
fields = { file = 1, line = 2, column = 3 }

# error[Exxxx]: message
regex = '^error\[E(\d+)\]: (.+)$'
fields = { code = 1, message = 2 }

# 缩进 + 行号 + 级别 + 消息
regex = '^\s+(\d+):(\d+)\s+(error|warning)\s+(.+?)\s+(\S+)$'
fields = { line = 1, column = 2, severity = 3, message = 4, code = 5 }
```

## Stateful 模式

当 `parser_type = "stateful"` 时，pattern 支持跨行状态机：

```toml
[meta]
name = "npm"
parser_type = "stateful"
detect = ["npm"]

[[pattern]]
name = "npm-error"
regex = '^npm ERR! (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { message = 1 }
state_transition = "has_error=true"    # 匹配后设置状态

[[pattern]]
name = "npm-added"
regex = '^added (\d+) packages?'
event_type = "summary"
severity = "info"
state_condition = "has_error=false"    # 仅在未出错时匹配
state_transition = "packages_added=done"
```

### 状态条件

- `state_condition = "key=value"`：仅当状态 `key` 等于 `value` 时匹配
- `state_transition = "key=value"`：匹配成功后设置状态
- 状态在任务生命周期内持久，跨行共享

## 完整示例

```toml
[meta]
name = "eslint"
description = "ESLint linter"
detect = ["eslint"]
priority = 50

[[pattern]]
name = "eslint-diagnostic"
regex = '^\s+(\d+):(\d+)\s+(error|warning)\s+(.+?)\s+(\S+)$'
event_type = "diagnostic"
severity = "error"
fields = { line = 1, column = 2, severity = 3, message = 4, code = 5 }

[[pattern]]
name = "eslint-location"
regex = '^(.+?):(\d+):(\d+)$'
event_type = "location"
severity = "info"
fields = { file = 1, line = 2, column = 3 }
```

## 加载与热重载

1. 启动时加载内置 + 用户 parser
2. 用户 parser 修改后自动重载（`parser.hot_reload = true`）
3. 无效正则跳过并记录警告
4. TOML 语法错误跳过并记录错误
