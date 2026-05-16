# Daemon 管理

## 启停

```bash
arshy daemon start      # 启动（已运行则报错）
arshy daemon stop       # 优雅关闭
arshy daemon restart    # stop + start
```

## 自动启动

默认 `auto_start = true`。执行 `arshy run` 或 MCP proxy 连接时，daemon 未运行会自动启动。

## PID 文件

路径：`~/.local/share/arshy/arshyd.pid`

- 启动时写入 PID，防止重复启动
- 退出时自动删除
- 残留 PID 文件会检查进程是否存活

## Socket 清理

路径：`~/.local/share/arshy/arshyd.sock`

- 启动时清理残留 socket（对应进程已不存在）
- 退出时自动删除

## 信号处理

| 信号 | 行为 |
|------|------|
| `SIGINT` (Ctrl-C) | 优雅关闭：等待当前任务完成，清理资源 |
| `SIGTERM` | 同上 |
| `daemon/shutdown` RPC | 同上 |

## 系统服务（可选）

### macOS launchd

创建 `~/Library/LaunchAgents/com.arshy.daemon.plist`：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.arshy.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/arshyd</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/com.arshy.daemon.plist
```

### Linux systemd

创建 `~/.config/systemd/user/arshyd.service`：

```ini
[Unit]
Description=Arshy daemon

[Service]
ExecStart=/usr/local/bin/arshyd
Restart=on-failure

[Install]
WantedBy=default.target
```

```bash
systemctl --user enable --now arshyd
```

## 状态查看

```bash
arshy status            # daemon 运行状态
arshy stats             # 任务统计（P50/P99/失败率）
```
