# 管理 arshy daemon

本文解决以下任务：启动/停止/重启 daemon、查看状态、理解空闲自动退出与按需自启、可选注册 launchd/systemd、用 `self-update` 更新二进制。

daemon（`arshyd`）是后台进程，负责 PTY 命令执行、输出解析、JSONL 存储与事件推送；CLI/proxy（`arshy`）通过 Unix Domain Socket 与它通信。daemon 生命周期实现见 `src/daemon/main.rs`、`src/daemon/lifecycle.rs`，CLI 侧见 `src/cli/mod.rs`。

## 1. 启动 daemon

### 按需自启（默认方式）

daemon 默认**按需自启**（`daemon.auto_start = true`）：第一次执行命令时自动拉起，无需任何 OS 级注册：

```bash
arshy run "echo hello"      # 首次运行会先自动启动 arshyd
```

按需自启发生在三个入口（`src/proxy/connection.rs` 的 `connect_or_start` / `start_daemon`）：

- `arshy run ...`（CLI）
- `arshy --from-mcp`（MCP proxy，agent 调用 `arshy_exec` 时）
- bash 代理（`bash -c` 被 `~/.arshy/bin/bash` shim 拦截时）

自启保证（`start_daemon`）：

1. **兄弟二进制**：优先解析当前 `arshy` 可执行文件旁边（`canonicalize` 后）的 `arshyd`，避免 PATH 上旧版本；
2. **会话脱离**：`setsid()` 使 daemon 独立于调用方会话（agent 工具调用、CI 步骤结束后存活）；
3. **spawn 锁**：`/tmp/arshyd.spawn-lock` 原子创建，防止并发重复拉起；
4. **熔断器**：2 分钟内崩溃 5 次后抑制自动拉起（详见《排查问题》）。

### 手动启动

```bash
arshy daemon start
```

期望输出：

- daemon 未运行：`daemon started`（命令最多等待 5 秒直到 socket 可连接，超时返回 `daemon unreachable: daemon did not start within 5s`）。
- daemon 已在运行：`daemon is already running`（幂等）。

关闭自动启动（例如 CI 环境）后，必须手动启动：

```bash
export ARSHY_DAEMON_AUTO_START=false   # 或写配置文件 daemon.auto_start = false
arshy daemon start                     # 手动拉起
```

## 2. 查看 daemon 状态

顶层命令 `arshy status`（注意：**没有** `arshy daemon status` 子命令，`daemon` 只有 `start`/`stop`/`restart`）：

```bash
arshy status
```

输出 JSON：

```json
{
  "uptime_secs": 22,
  "tasks_running": 0,
  "tasks_total": 185,
  "db_size_bytes": 491043,
  "counters": {
    "connections_accepted": 31,
    "events_emitted": 0,
    "tasks_completed": 12,
    "tasks_created": 13,
    "tasks_failed": 4
  }
}
```

daemon 未运行时 `arshy status` 会报错（socket 不存在）：

```
Error: Io(Os { code: 2, kind: NotFound, message: "No such file or directory" })
```

运行中的标识文件（`src/daemon/lifecycle.rs`）：

- PID 文件：`${XDG_DATA_HOME}/arshy/arshyd.pid`（默认 `~/.local/share/arshy/arshyd.pid`）；
- socket：`${XDG_DATA_HOME}/arshy/arshyd.sock`，权限 `0600`，并校验连接方 UID 与 daemon 相同（`src/daemon/main.rs` 的 `peer_uid_allowed`）。

## 3. 停止与重启 daemon

```bash
arshy daemon stop       # 优雅关闭
arshy daemon restart    # 停止（若未运行则忽略错误）+ 500ms 后启动
```

`stop` 通过 IPC 发送 shutdown 请求，期望输出：

```json
{ "status": "shutting_down" }
```

优雅关闭流程（`src/daemon/main.rs`）：

1. 广播 `DaemonShutdown` 事件（30 秒宽限提示）；
2. 删除 socket，拒绝新连接；
3. 等待运行中任务自然完成（最多 5 秒）；
4. 5 秒后对剩余任务执行进程树终止（`kill_all`）；
5. 硬截止时间 30 秒，超时强制退出；
6. 清理：flush store、删除 PID 文件与 socket、记录 `arshyd stopped`。

重启后确认：

```bash
arshy daemon restart
arshy status        # uptime_secs 归零
```

## 4. 空闲自动退出

daemon 默认在空闲 15 分钟后自我退出，防止 IDE 关闭后进程常驻（`daemon.idle_timeout_secs = 900`）：

- 判定条件：**没有运行中的任务**，且**最近完成的任务早于空闲阈值**（`store.idle_since_secs`）；
- 检查周期随阈值缩放：`min(idle, 120)/2` 秒，下限 5 秒、上限 60 秒；
- `idle_timeout_secs = 0` 完全禁用（进程常驻，直到被 stop/关机）。

```toml
# ~/.config/arshy/config.toml
[daemon]
idle_timeout_secs = 3600   # 空闲 1 小时退出
# idle_timeout_secs = 0    # 禁用
```

环境变量方式：`ARSHY_DAEMON_IDLE_TIMEOUT_SECS=0`（非 0 值会被钳制为至少 30 秒）。

配合按需自启，闲置的 daemon 消失后，下一次命令会再次自动拉起——这是设计上的"零常驻"模式。

## 5. 可选：注册 launchd / systemd

### 现状：默认不注册 OS 级服务

arshy 有意**不做** OS 级自启注册（`install/install.sh` 明确说明："on-demand auto-start by design … no OS-level registration"，CLI 中也不存在 `install-launchd`/`install-systemd` 子命令）。这样开机不会残留进程。

唯一由 `arshy setup` 写入的 launchd 文件是 `~/Library/LaunchAgents/com.arshy.path.plist`——它只负责登录时把 `~/.arshy/bin` 加回 GUI 会话 PATH（GUI PATH 层，见《把 arshy 接入 AI Agent》），**不管理 daemon 进程**。

### 需要常驻时的手工注册

如果你希望 daemon 常驻（例如关闭空闲退出并让 launchd 在崩溃后自动拉起），可以手工创建 plist。daemon 的 panic 钩子与 proxy 的重试逻辑都假设了这类 KeepAlive 场景（`src/daemon/main.rs`：`The daemon will restart automatically (KeepAlive)`；`src/proxy/mod.rs`：`daemon may be restarting (launchd KeepAlive)`）。

示例 `~/Library/LaunchAgents/com.arshy.daemon.plist`（macOS）：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.arshy.daemon</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/me/.local/bin/arshyd</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/Users/me/.local/share/arshy/daemon.log</string>
  <key>StandardErrorPath</key><string>/Users/me/.local/share/arshy/daemon.log</string>
</dict>
</plist>
```

```bash
launchctl load -w ~/Library/LaunchAgents/com.arshy.daemon.plist
```

systemd（Linux 用户级服务 `~/.config/systemd/user/arshy.service`）：

```ini
[Unit]
Description=Arshy daemon

[Service]
ExecStart=%h/.local/bin/arshyd
Restart=always
StandardOutput=append:%h/.local/share/arshy/daemon.log
StandardError=append:%h/.local/share/arshy/daemon.log

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now arshy
```

注意：常驻模式下建议设置 `daemon.idle_timeout_secs = 0`，否则空闲后 daemon 仍会自行退出（KeepAlive 会再拉起，但会造成反复启停）。

### 日志落盘

daemon 日志走 stderr（`log_format = "text"` 为 compact 格式，`"json"` 为 JSON Lines）。自动拉起时 stderr 被丢弃，所以：

- 手动调试：`./target/debug/arshyd 2>&1 | tee daemon.log`；
- launchd/systemd：用上面的 `StandardOutPath`/`StandardErrorPath` 落盘；
- 错误提示里引用的 `~/.local/share/arshy/daemon.log` 是"应有日志的位置"：只有当 daemon 以重定向 stderr 的方式运行（或 launchd 配置如上）时该文件才存在。

## 6. 用 self-update 更新二进制

`arshy self-update` 把**当前运行的构建**（连同旁边的 `arshyd`）复制到安装目录（`src/cli/mod.rs` 的 `self_update`）：

```bash
cargo build --release
arshy self-update
```

行为：

- 默认目标目录：开发构建（路径含 `target/`）→ `~/.local/bin`；已安装的二进制 → 其自身所在目录（此时为 no-op）；
- `--dest <dir>` 覆盖目标目录；
- 复制后显式设置 `0755` 可执行权限；
- 输出：
  ```
    ✓ updated /Users/me/.local/bin/arshy
    ✓ updated /Users/me/.local/bin/arshyd
  Restart the daemon to pick up the new binary: `arshy daemon restart`
  ```

更新后必须重启 daemon 才会加载新二进制：

```bash
arshy self-update --dest ~/.local/bin
arshy daemon restart
arshy --version && arshyd --version   # 两个版本号应一致
```

macOS 注意：手工 `cargo build --release && cp` 之后**不会自动重新签名**。安装脚本会在复制后执行 ad-hoc 签名（`codesign --force --deep -s -`），防止 macOS 静默 SIGKILL 绑定 socket 的 daemon（详见《排查问题》）。如果你手工替换了二进制，请补一步：

```bash
codesign --force --deep -s - ~/.local/bin/arshy ~/.local/bin/arshyd
```

## 7. 安装与卸载时的 daemon 处理

- `install/install.sh`：安装到 `~/.local/bin`，随后 `arshy hook install` 安装 shim；不注册 OS 级自启（第 5 节）。
- `arshy install`：注册 MCP 后会检查 socket，未运行则自动拉起 daemon（最多等 3 秒，失败提示 `arshy daemon start`）。
- `arshy uninstall`：移除注册与 shim；**不**删除任务数据（提示保留在 `~/.local/share/arshy`，可手工删除）。
