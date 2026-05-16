# 故障排查

## daemon 无法启动

**"daemon already running (pid xxx)"**

PID 文件残留但进程已死：

```bash
rm ~/.local/share/arshy/arshyd.pid
arshy daemon start
```

**"Address already in use"**

Socket 文件残留：

```bash
rm ~/.local/share/arshy/arshyd.sock
arshy daemon start
```

## Agent 无法执行命令

**"daemon unreachable"**

daemon 未运行：

```bash
arshy daemon start
```

**"access denied: read-only mode"**

安全策略限制：

```bash
arshy config set security.access_level full
arshy daemon restart
```

**"command blocked"**

命令被安全过滤器拦截。检查 `security.blocked_patterns` 或 `security.allowed_commands`。

## Parser 不生效

**自定义 parser 未加载**

1. 确认文件在 `~/.arshy/parsers/` 下（`.toml` 或 `.rhai`）
2. 检查 daemon 日志：`arshy daemon restart`，观察是否有 `parsers reloaded`
3. TOML 语法错误会在日志中报告

**命令未匹配到 parser**

`detect` 列表必须包含命令首词的子串。`detect = ["cargo"]` 匹配 `cargo build` 但不匹配 `rustc`。

## 输出未结构化

**短命令返回纯文本**

正常行为。`mode = "auto"` 时，短命令（如 `ls`、`echo`）走零开销路径，不经过 parser。

**长命令也返回纯文本**

检查 parser 是否匹配了该工具：`arshy query <task_id>` 查看是否有结构化事件。

## 数据库问题

**磁盘空间不足**

```bash
arshy prune --older-than 7   # 删除 7 天前的任务
arshy stats                   # 查看数据库大小
```

**数据库损坏**

```bash
arshy daemon stop
sqlite3 ~/.local/share/arshy/arshy.db "PRAGMA integrity_check;"
```

## 性能

**命令执行慢**

检查 `daemon.max_task_duration_ms`（默认 3600000 = 1 小时）。超时任务会被自动终止。

**通知延迟**

调整 `notifications.batch_interval_ms`（默认 100ms）。设为 0 禁用批处理。
