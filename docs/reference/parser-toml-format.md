# TOML Parser 格式参考（Schema v1.0）

## 文件位置

| 来源 | 路径 | 说明 |
|------|------|------|
| 内置 | 编译时嵌入 | 20 个 builtin parser（31 pattern） |
| 用户 | `~/.arshy/parsers/*.toml` | 自定义 parser，优先级 > 内置 |

同名用户 parser 覆盖内置。热重载自动检测文件变更并输出 diff 审计日志。

## 完整结构

```toml
[meta]
name = "tsc"
description = "TypeScript compiler"
detect = ["tsc"]              # 匹配第一词（starts_with）
detect_full = ["npx tsc"]     # 匹配完整命令（starts_with）
parser_type = "toml"          # "toml"（默认）或 "stateful"
priority = 50
schema_version = "1.0"        # schema 版本
since_version = "0.1.0"       # 引入版本（可选）
min_version = "4.0.0"         # 工具最低版本（可选，semver）
max_version = "6.0.0"         # 工具最高版本（可选，semver）

[[pattern]]
name = "ts-error"
regex = '^(.+?)\((\d+),(\d+)\): error TS(\d+): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, column = 3, code = 4, message = 5 }
deprecated = false            # 弃用标记（可选，默认 false）
replaced_by = "ts-error-v2"   # 替代 pattern 名称（可选）
since_version = "0.1.0"       # 引入版本（可选）

# stateful-only 字段：
state_condition = "state=value"    # 匹配条件（可选）
state_transition = "key=value"     # 状态转移（可选）
```

## [meta] 字段

| 字段 | 类型 | 必填 | 默认 | 说明 |
|------|------|:--:|------|------|
| `name` | string | ✅ | — | parser 唯一名称 |
| `description` | string | | — | 描述 |
| `detect` | string[] | ✅ | — | 命令匹配词（与第一词 starts_with 匹配） |
| `detect_full` | string[] | | `[]` | 完整命令匹配（与完整命令 starts_with 匹配） |
| `parser_type` | string | | `"toml"` | `"toml"` 或 `"stateful"` |
| `priority` | integer | | 50 | 优先级，高值优先匹配 |
| `schema_version` | string | | `"1.0"` | 此 parser 的 schema 版本 |
| `since_version` | string | | — | 此 parser 引入的 arshy 版本 |
| `min_version` | string | | — | 工具最低支持版本（semver，含） |
| `max_version` | string | | — | 工具最高支持版本（semver，含） |

## [[pattern]] 字段

| 字段 | 类型 | 必填 | 默认 | 说明 |
|------|------|:--:|------|------|
| `name` | string | ✅ | — | pattern 唯一名称 |
| `regex` | string | ✅ | — | 正则表达式（编译时经 ReDoS 安全校验） |
| `event_type` | string | ✅ | — | 事件类型：diagnostic / location / test_result / summary / crash |
| `severity` | string | ✅ | — | 严重度：error / warning / info |
| `fields` | map | | `{}` | 捕获组映射（file/line/column/code/message/severity） |
| `deprecated` | bool | | false | 标记弃用；加载时 log warn |
| `replaced_by` | string | | — | 替代 pattern 名；弃用且存在替代时自动跳过 |
| `since_version` | string | | — | 此 pattern 引入的 arshy 版本 |
| `state_condition` | string | | — | stateful-only：匹配前置条件（`"key=value"`） |
| `state_transition` | string | | — | stateful-only：匹配后状态转移（`"key=value"`） |

## fields 映射

| 键 | 说明 | 示例 |
|----|------|------|
| `file` | 文件路径（捕获组索引） | `file = 1` |
| `line` | 行号（捕获组索引） | `line = 2` |
| `column` | 列号（捕获组索引） | `column = 3` |
| `code` | 错误码（捕获组索引） | `code = 4` |
| `message` | 诊断消息（捕获组索引） | `message = 5` |
| `severity` | 动态严重度覆盖（捕获组索引） | `severity = 3` |

## ReDoS 安全校验

所有正则编译时自动检测 ReDoS 漏洞：
- **嵌套量词**：`(a+)+`、`(a*)*`、`(.+)+` → 拒绝加载
- **重叠交替**：`(a|ab)+b` → 拒绝加载

校验失败时 pattern 被跳过并记录 warn 日志，不影响其他 pattern。

## Pattern 生命周期

```
引入 (since_version) → 正常使用 → 弃用 (deprecated=true, replaced_by="...")
                                       │
                                       ├─ 有替代：自动跳过，使用替代 pattern
                                       └─ 无替代：继续使用，每次匹配 log warn
```
