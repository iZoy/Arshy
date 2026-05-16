# CLI 命令

## 全局选项

```
arshy [OPTIONS] [COMMAND]

--from-mcp          以 MCP stdio proxy 模式运行
--config <PATH>      配置文件路径
--log-level <LEVEL>  日志级别：trace/debug/info/warn/error
```

## 子命令

### arshy run

执行 shell 命令。

```
arshy run <COMMAND>
  --cwd <PATH>          工作目录
  --timeout-ms <MS>     超时时间
  --mode <MODE>         auto（默认）| sync | async
```

- `auto`：短命令同步返回，长命令异步执行
- `sync`：阻塞等待完成
- `async`：立即返回 task_id

### arshy list

列出任务。

```
arshy list
  --status <STATUS>     过滤：running/completed/failed/killed
  --limit <N>           最大条数（默认 10）
```

### arshy query

查询任务事件。

```
arshy query <TASK_ID>
  --event-type <TYPE>   过滤事件类型
  --severity <SEV>      过滤严重度：error/warning/info
  --code <CODE>         过滤错误码
  --file <FILE>         过滤文件路径
  --limit <N>           最大条数（默认 20）
```

### arshy kill

终止运行中的任务。

```
arshy kill <TASK_ID>
```

优雅终止：SIGINT → SIGTERM（2s）→ SIGKILL（2s）

### arshy tail

查看任务输出。

```
arshy tail <TASK_ID>
  --lines <N>           行数（默认 50）
  --format <FMT>        event（默认）| raw
```

### arshy stats

查看聚合统计。

```
arshy stats
```

输出：总任务数、状态分布、P50/P99 耗时、失败率、数据库大小。

### arshy prune

清理历史数据。

```
arshy prune
  --keep <N>            保留最近 N 条
  --older-than <DAYS>   删除 N 天前的数据
```

### arshy config

管理配置。

```
arshy config get <KEY>          获取配置值
arshy config set <KEY> <VALUE>  设置配置值
arshy config list               列出所有配置
arshy config path               显示配置文件路径
```

### arshy daemon

管理 daemon 进程。

```
arshy daemon start      启动
arshy daemon stop       停止
arshy daemon restart    重启
```

### arshy status

显示 daemon 状态：运行状态、任务数、连接数。

### arshy install / uninstall

注册/注销 MCP server（Claude Code、Cursor）。

### arshy install-launchd

安装 macOS launchd plist，使 daemon 开机自启。

```
arshy install-launchd
→ 安装到 ~/Library/LaunchAgents/com.arshy.daemon.plist
→ launchctl load/unload 管理
```

### arshy install-systemd

安装 Linux systemd user unit，使 daemon 开机自启。

```
arshy install-systemd
→ 安装到 ~/.config/systemd/user/arshyd.service
→ systemctl --user enable/start 管理
```
