# 自定义 Parser（TOML）

## 创建 Parser 文件

在 `~/.arshy/parsers/` 下创建 `.toml` 文件：

```toml
# ~/.arshy/parsers/mytool.toml

[meta]
name = "mytool"
description = "My custom tool"
detect = ["mytool"]
priority = 50

[[pattern]]
name = "error-line"
regex = '^(.+?):(\d+): error: (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, message = 3 }

[[pattern]]
name = "warning-line"
regex = '^(.+?):(\d+): warning: (.+)$'
event_type = "diagnostic"
severity = "warning"
fields = { file = 1, line = 2, message = 3 }
```

保存后 daemon 自动热重载（`hot_reload = true`），无需重启。

## 最小定义

只需 3 个字段：

```toml
[meta]
name = "mycli"
detect = ["mycli"]

[[pattern]]
name = "catch-all"
regex = '(?i)(error|fail)'
event_type = "diagnostic"
severity = "error"
```

## 检测规则

`detect` 列表中的字符串与命令首词做**子串匹配**（不区分大小写）。

```
detect = ["cargo"]    → 匹配 "cargo build", "cargo test", "cargo-clippy"
detect = ["docker"]   → 匹配 "docker build", "docker-compose up"
```

优先级：数字越大越优先。同名 parser 中用户定义覆盖内置。

## 字段映射

`fields` 将正则捕获组映射到事件字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `file` | string | 文件路径（捕获组索引） |
| `line` | integer | 行号 |
| `column` | integer | 列号 |
| `message` | string | 消息文本 |
| `code` | string | 错误码 |

值为正则捕获组编号（从 1 开始）。

## 状态化 Parser

设置 `parser_type = "stateful"` 启用跨行状态机：

```toml
[meta]
name = "mybuilder"
detect = ["mybuilder"]
parser_type = "stateful"

[[pattern]]
name = "error"
regex = '^ERROR (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { message = 1 }
state_transition = "has_error=true"

[[pattern]]
name = "summary"
regex = '^Build: (\d+) errors'
event_type = "summary"
severity = "info"
fields = { message = 1 }
state_condition = "has_error=true"
```

状态机通过 `state_condition` 控制匹配条件，`state_transition` 设置状态值。

## 测试 Parser

1. 创建 fixture 文件：`parsers/builtin/tests/mytool/test.txt`（输入行）+ `test.json`（期望事件）
2. 运行：`cargo test fixture_mytool`

JSON fixture 格式：

```json
[
  {"type": "diagnostic", "severity": "error", "file": "main.rs", "line": 42},
  {"type": "summary", "severity": "info"}
]
```

省略的字段不做断言（通配）。

完整格式参考：[Parser TOML 格式](../reference/parser-toml-format.md)
