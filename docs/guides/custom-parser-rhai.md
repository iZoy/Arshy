# 自定义 Parser（Rhai）

当 TOML 正则无法表达跨行逻辑时，使用 Rhai 脚本。

## 创建脚本

在 `~/.arshy/parsers/` 下创建 `.rhai` 文件：

```rhai
// ~/./.arshy/parsers/docker.rhai

fn on_line(line, ctx) {
    if line.contains("ERROR") {
        ctx.emit("diagnostic", "error", line);
        ctx.set("has_error", true);
    }
    if line.starts_with("WARNING") {
        ctx.emit("diagnostic", "warning", line);
    }
    if line.starts_with("Successfully built") {
        ctx.emit("summary", "info", line);
    }
}

fn on_complete(exit_code, ctx) {
    if exit_code != 0 {
        ctx.emit("summary", "error", "failed (exit " + exit_code + ")");
    }
}
```

文件名即 parser 名：`docker.rhai` → 检测命令中的 `docker`。

## API

### 回调函数

| 函数 | 触发时机 | 参数 |
|------|----------|------|
| `on_line(line, ctx)` | 每行输出 | `line: string`, `ctx: Ctx` |
| `on_complete(exit_code, ctx)` | 命令结束 | `exit_code: int`, `ctx: Ctx` |

### ctx 方法

| 方法 | 说明 |
|------|------|
| `ctx.emit(event_type, severity, message)` | 发射事件 |
| `ctx.emit(event_type, severity, message, file, line)` | 带位置的事件 |
| `ctx.set(key, value)` | 写状态（跨行持久） |
| `ctx.get(key) -> value` | 读状态（无值返回 `()`） |
| `ctx.has(key) -> bool` | 状态是否存在 |

### 事件类型

| event_type | 说明 |
|------------|------|
| `diagnostic` | 编译错误/警告/lint |
| `test_result` | 测试通过/失败 |
| `summary` | 命令级汇总 |
| `log` | 通用日志 |
| `location` | 文件位置信息 |
| `crash` | 崩溃/异常 |

### severity

`"error"` | `"warning"` | `"info"` | `"debug"`

## 状态持久化

`ctx.set/get/has` 的状态在同一次命令执行的所有行间持久。

```rhai
fn on_line(line, ctx) {
    if line.contains("error") {
        let count = ctx.get("error_count");
        if count == () { count = 0; }
        ctx.set("error_count", count + 1);
    }
}

fn on_complete(exit_code, ctx) {
    let count = ctx.get("error_count");
    if count != () {
        ctx.emit("summary", "error", count + " errors");
    }
}
```

## 带位置的事件

```rhai
fn on_line(line, ctx) {
    // 匹配 "src/main.rs:42: something wrong"
    if line.contains(":") {
        ctx.emit("diagnostic", "error", line, "src/main.rs", 42);
    }
}
```

## 语法校验

脚本在加载时编译校验。语法错误会在 daemon 日志中报告：

```
rhai script 'docker': rhai compile error: ...
```

修正后自动热重载。

## 与 TOML 的选择

| 场景 | 选择 |
|------|------|
| 逐行正则匹配 | TOML |
| 跨行状态机 | TOML `parser_type = "stateful"` |
| 复杂条件逻辑、计数器、上下文感知 | Rhai |
| 需要字符串方法（contains/starts_with） | Rhai |

示例文件参考：`parsers/examples/docker.rhai`、`parsers/examples/kubectl.rhai`

完整 API 参考：[Parser Rhai API](../reference/parser-rhai-api.md)
