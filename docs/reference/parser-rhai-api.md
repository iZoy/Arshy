# Rhai Parser API 参考

## 概述

Rhai parser 使用 [Rhai 脚本语言](https://rhai.rs) 编写，适用于需要跨行状态追踪的复杂输出格式。

## 文件位置

| 来源 | 路径 |
|------|------|
| 用户 | `~/.arshy/parsers/*.rhai` |
| 示例 | `parsers/examples/*.rhai` |

文件名（不含扩展名）作为 parser 名称和 detect 词。如 `docker.rhai` → 名称 `docker`，匹配命令首词 `docker`。

## 必须定义的函数

### on_line(line, ctx)

每行输出调用一次。

| 参数 | 类型 | 说明 |
|------|------|------|
| `line` | string | 当前行文本 |
| `ctx` | RhaiCtx | 上下文对象 |

### on_complete(exit_code, ctx)

命令执行完成后调用一次。

| 参数 | 类型 | 说明 |
|------|------|------|
| `exit_code` | int | 进程退出码 |
| `ctx` | RhaiCtx | 上下文对象 |

## ctx 方法

### emit(type, severity, message)

发送一个事件。

| 参数 | 类型 | 说明 |
|------|------|------|
| `type` | string | 事件类型 |
| `severity` | string | `error` / `warning` / `info` |
| `message` | string | 消息文本 |

```rhai
ctx.emit("diagnostic", "error", "compilation failed");
```

### emit(type, severity, message, file, line)

发送带位置信息的事件。

| 参数 | 类型 | 说明 |
|------|------|------|
| `type` | string | 事件类型 |
| `severity` | string | 严重度 |
| `message` | string | 消息文本 |
| `file` | string | 文件路径 |
| `line` | int | 行号 |

```rhai
ctx.emit("diagnostic", "error", "undefined variable", "src/main.rs", 42);
```

### set(key, value)

设置持久状态值（跨行共享，任务生命周期内有效）。

| 参数 | 类型 | 说明 |
|------|------|------|
| `key` | string | 状态键名 |
| `value` | any | 值（string / int / bool） |

```rhai
ctx.set("error_count", 0);
ctx.set("in_block", true);
```

### get(key)

读取状态值。未设置时返回 `()`（unit）。

```rhai
let count = ctx.get("error_count");
if count == () {
    count = 0;
}
```

### has(key)

检查状态键是否存在。返回 `bool`。

```rhai
if ctx.has("seen_error") {
    // ...
}
```

## 事件类型

| type | 说明 |
|------|------|
| `diagnostic` | 编译/lint 诊断 |
| `location` | 文件位置 |
| `test_result` | 测试结果 |
| `summary` | 汇总（通常在 on_complete 中使用） |
| `log` | 通用日志 |

## 支持的 Rhai 语言特性

- 变量声明：`let x = 42;`
- 控制流：`if/else`、`while`、`loop`、`for`
- 字符串操作：`.contains()`、`.starts_with()`、`.len()`、`.trim()`
- 数组：`[1, 2, 3]`、`.len()`、`.push()`
- 闭包：`|x| x + 1`
- 类型检测：`type_of(x)`

**不支持**：外部模块导入、文件 I/O、网络、正则表达式（在 Rhai 层面）。

> 如果需要正则匹配，使用 TOML parser（`parser_type = "stateful"`）。

## 完整示例

```rhai
fn on_line(line, ctx) {
    // 检测错误
    if line.contains("ERROR") || line.contains("error:") {
        ctx.emit("diagnostic", "error", line);
        let count = ctx.get("error_count");
        if count == () {
            ctx.set("error_count", 1);
        } else {
            ctx.set("error_count", count + 1);
        }
    }

    // 检测警告
    if line.contains("WARN") || line.contains("warning") {
        ctx.emit("diagnostic", "warning", line);
    }

    // 检测构建阶段
    if line.starts_with("Step") {
        ctx.set("current_stage", line);
    }
}

fn on_complete(exit_code, ctx) {
    let count = ctx.get("error_count");
    if count == () {
        count = 0;
    }

    if exit_code == 0 {
        ctx.emit("summary", "info", "build succeeded");
    } else {
        ctx.emit("summary", "error",
            "build failed with " + count + " errors");
    }
}
```

## 错误处理

- **语法错误**：加载时报错，parser 不注册
- **运行时错误**：`on_line` / `on_complete` 中的错误被吞没并记录 debug 日志
- **未定义函数**：如果缺少 `on_line` 或 `on_complete`，对应调用静默跳过
