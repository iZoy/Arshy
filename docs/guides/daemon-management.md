# Daemon 管理

## 启动

Daemon 按需启动，无需手动干预。首次 `arshy_exec` 调用时 proxy 自动 spawn daemon。

手动管理：

```bash
arshy daemon start     # 启动（如未运行）
arshy daemon stop      # 优雅关闭
arshy daemon restart   # 重启
arshy status           # 查看状态
```

## 空闲退出

Daemon 在 5 分钟无活动后自动退出以节省资源。proxy 保持运行，下次 `arshy_exec` 调用时自动重启 daemon。

自定义超时：

```bash
export ARSHY_IDLE_TIMEOUT_SECS=600  # 10 分钟
```

## 进程树关闭

终止任务时，`graceful_kill` 发送信号到进程组（负 PID）+ 直接 PID，捕获 shell 管道和构建工具 fork 的子进程。

信号升级：SIGINT → wait → SIGTERM → wait → SIGKILL。

Daemon 关闭时的 drain 流程：
1. 移除 socket（拒绝新连接）
2. 发布 `DaemonShutdown` 通知
3. 5 秒自然等待运行中任务完成
4. 调用 `executor.kill_all()` 主动终止剩余任务（进程组信号）
5. 30 秒硬截止后强制退出

## 健康检查

Proxy 每 30 秒空闲时检查 daemon 健康状态。连续失败时指数退避：1s → 2s → 4s → 8s → ... → 60s 上限。防止多个 proxy 同时重连造成雷群。

## 生命周期文件

| 文件 | 用途 |
|------|------|
| `~/.local/share/arshy/arshyd.sock` | Unix socket（0600） |
| `~/.local/share/arshy/arshy.db` | SQLite 数据库 |
| `~/.local/share/arshy/arshyd.pid` | 进程 PID |
| `/tmp/arshyd.spawn-lock` | 防止重复 spawn 的原子锁（自动清理） |

## 崩溃保护

- **Circuit Breaker**：2 分钟内 crash 超过 5 次 → 抑制自动重启
- **Panic Hook**：全局 panic 拦截器，进程退出前记录消息和位置
- **Spawn Lock**：`/tmp/arshyd.spawn-lock` 确保只有一个 proxy 在 spawn
- **连接限制**：`Semaphore(64)` 限制并发连接数
- **Telemetry**：原子计数器（tasks/events/connections）暴露在 stats/health 端点

## 开机自启（可选）

macOS：

```bash
arshy install-launchd
```

Linux：

```bash
arshy install-systemd
```
