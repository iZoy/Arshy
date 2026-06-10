# CLI 命令

## 全局选项

```
arshy [OPTIONS] [COMMAND]

--from-mcp          以 MCP stdio proxy 模式运行。proxy 连接 arshyd daemon 并通过 Unix socket
                    转发 MCP JSON-RPC 请求。用于 Claude Code / Cursor 集成
--config <PATH>      配置文件路径。不设置时使用默认路径，支持 env var 覆盖
--log-level <LEVEL>  日志级别：trace/debug/info/warn/error。覆盖配置文件中的值
```

## 子命令

### arshy run

执行 shell 命令。

```
arshy run <COMMAND>
  --cwd <PATH>          工作目录（绝对路径）
  --timeout-ms <MS>     超时时间（ms）
  --mode <MODE>         auto（默认）| sync | async
  --format <FMT>        输出格式：pretty（终端 UI）| json（原始 JSON）| auto（默认，TTY 检测）
  --errors-only         只返回 error 级别事件（过滤 warning/info）
```

- `auto` 模式：80+ 条规则智能判断短/长命令。短命令同步返回原文，长命令异步执行
- `sync`：阻塞等待完成，返回完整结构化结果
- `async`：立即返回 task_id，后台执行
- `pretty` 格式：box drawing + ANSI color 可视化输出

### arshy list

列出任务。

```
arshy list
  --status <STATUS>     过滤：running/completed/failed/killed
  --limit <N>           最大条数（默认 10）
```

### arshy query

查询任务的结构化事件。

```
arshy query <TASK_ID>
  --event-type <TYPE>   过滤事件类型（diagnostic/location/test_result/summary/crash）
  --severity <SEV>      过滤严重度：error/warning/info
  --code <CODE>         过滤错误码（如 E0308、TS2345）
  --file <FILE>         过滤文件路径
  --limit <N>           最大条数（默认 20）
```

### arshy kill

终止运行中的任务。发送进程组信号：SIGINT → SIGTERM（可配置）→ SIGKILL。

```
arshy kill <TASK_ID>
```

### arshy tail

查看任务输出。

```
arshy tail <TASK_ID>
  --lines <N>           行数（默认 50）
  --format <FMT>        event（默认，结构化）| raw（完整原始文本，来自 raw_output 列）
```

### arshy status

显示 daemon 状态：运行状态、任务数、连接数、遥测计数器。

### arshy stats

聚合统计：总任务数、状态分布、数据库大小、Token 节省量。

输出示例：
```
┌─ arshy stats ─────────────────────────────────────┐
│ Tasks:     1,247 total (1,180 ok / 67 failed)     │
│ Events:    18,432 (892 errors)                     │
│ Duration:  p50=1.2s  p99=45.3s                     │
│                                                    │
│ Token savings:  ~73% (est. 890K → 240K tokens)     │
│ Parser coverage: 82% lines matched parsers         │
│ DB size:        12.4 MB                            │
└────────────────────────────────────────────────────┘
```

### arshy prune

清理历史数据。

```
arshy prune
  --keep <N>            保留最近 N 条任务
  --older-than <DAYS>   删除 N 天前的数据（最小 1 天）
```

### arshy config

管理配置。

```
arshy config get <KEY>          获取配置值（如 daemon.log_level）
arshy config set <KEY> <VALUE>  设置配置值
arshy config list               列出所有配置
arshy config path               显示配置文件路径
```

### arshy daemon

管理 daemon 进程。

```
arshy daemon start      启动 daemon（如未运行）
arshy daemon stop       停止 daemon（发送 shutdown 请求）
arshy daemon restart    重启 daemon
```

### arshy install / uninstall

注册/注销 MCP server（写入/删除 `~/.claude/settings.json` 或 `.cursor/mcp.json`）。

### arshy install-launchd

安装 macOS launchd plist 到 `~/Library/LaunchAgents/com.arshy.daemon.plist`。

### arshy install-systemd

安装 Linux systemd user unit 到 `~/.config/systemd/user/arshyd.service`。

### arshy doctor

诊断 Claude Code 集成状态，显示修复建议。

### arshy benchmark

跨所有 37 个内置 parser 运行性能测试，输出：
- 信息密度（结构化字段/event）
- Token 效率（raw vs structured 压缩比）
- 错误定位速度
- 解析准确率

```
arshy benchmark
```

### arshy parser reload

热重载 parser 定义并显示变更 diff。用于编辑自定义 parser 后验证。

```
arshy parser reload
```

### arshy parser list

列出已加载的 parser。

```
arshy parser list
```
