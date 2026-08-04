# 排查 arshy 常见问题

本文解决以下任务：daemon 连不上/自动重启失败、macOS 上 daemon 被 SIGKILL、源码改动"不生效"（cargo 缓存假象）、socket 残留、spawn-lock 残留恢复。

所有结论来自代码事实（`src/cli/mod.rs`、`src/proxy/mod.rs`、`src/daemon/main.rs`、`src/daemon/lifecycle.rs`、`src/cli/integrate.rs` 等）与实测输出。

## 1. daemon 连不上（daemon unreachable / connection refused / No such file）

### 症状与快速判断

| 症状 | 常见原因 |
|---|---|
| `arshy status` 报 `Error: Io(Os { code: 2, kind: NotFound, message: "No such file or directory" })` | daemon 未运行（socket 不存在） |
| `arshy run` 报 `daemon unreachable: ...` | 自启失败（详见第 2 节） |
| `arshy run` 报 `Error: Io(... ConnectionRefused ...)` | daemon 已退出但 socket 残留（极少，正常启动会清） |

先确认 daemon 是否在跑：

```bash
ps -p "$(cat ~/.local/share/arshy/arshyd.pid 2>/dev/null)"   # PID 文件
./target/debug/arshyd --version                                # 版本自检（不会启动 daemon）
arshy status                                                    # 正常时返回 JSON 状态
```

### 修复步骤

1. 如果 socket 存在但连不上（daemon 已死），`arshy run` / proxy 的 `connect_or_start` 会自动删除残留 socket 再拉起；也可以手动清：
   ```bash
   rm -f ~/.local/share/arshy/arshyd.sock ~/.local/share/arshy/arshyd.pid
   ```
2. 手动启动：
   ```bash
   arshy daemon start
   ```
   - 输出 `daemon started`：正常；
   - 输出 `daemon is already running`：daemon 在跑，但客户端仍连不上时检查 socket 路径是否一致（`arshy config get daemon.socket_path`）；
   - 输出 `daemon unreachable: daemon did not start within 5s`：见第 2 节。

### 自启被禁用的情况

`daemon.auto_start = false`（或 `ARSHY_DAEMON_AUTO_START=false`）时，`arshy run` **不会**自动拉起 daemon，实测输出为：

```
Error: Io(Os { code: 2, kind: NotFound, message: "No such file or directory" })
```

修复：先手动 `arshy daemon start`，或把 `auto_start` 改回 `true`。MCP proxy 路径下的错误提示更友好（`src/proxy/protocol.rs` 的 `format_daemon_error`）：`Run arshy daemon start to start it.`

## 2. daemon 反复崩溃 / 自启被熔断

### 熔断器

`start_daemon` 内置熔断器（`src/proxy/connection.rs`）：**2 分钟内崩溃 5 次**后自动拉起被抑制，报错：

```
daemon unreachable: daemon crash-loop detected, auto-start suppressed
```

MCP proxy 会转成可读提示：

```
arshyd is crash-looping and auto-start has been suppressed.
Check the daemon logs: cat ~/.local/share/arshy/daemon.log
Then restart: arshy daemon restart
```

### 修复步骤

1. 先拿到崩溃原因。daemon 日志走 **stderr**：自动拉起时 stderr 被丢弃；手动运行可看到完整日志：
   ```bash
   ./target/debug/arshyd 2>&1 | tee /tmp/arshyd.log
   ```
   若用 launchd/systemd 常驻，日志在 plist/service 的 `StandardErrorPath` 指定的文件（提示文案里的 `~/.local/share/arshy/daemon.log` 即此类位置）。
2. 常见崩溃原因：
   - 配置错误（如 `sandbox_mode` 填了 `"none"`/`"workspace"` 之外的值，daemon 启动直接报 `Config` 错误）；
   - 配置文件 TOML 语法错误；
   - macOS 未签名导致 SIGKILL（见第 3 节）；
   - 端口/socket 目录权限问题（daemon 需要能创建 `${XDG_DATA_HOME}/arshy` 与 socket 父目录）。
3. 修复后重启并确认：
   ```bash
   arshy daemon restart
   arshy status
   ```

daemon 自身对"二次启动"也有保护：PID 文件显示已在运行时，新进程打印 `daemon already running (pid N)` 并以退出码 1 退出（`src/daemon/main.rs`）。

## 3. macOS：daemon 被 SIGKILL / codesign

### 背景

macOS 会对未正确签名的、绑定 socket 的二进制实施静默 SIGKILL。安装脚本与 CI 都做 **ad-hoc 签名**（`codesign --force --deep -s -`，见 `install/install.sh` 与 `.github/workflows/release.yml`）。

### 症状

- daemon 启动后立刻消失，日志无异常、无 panic；
- `arshy status` 连不上；`arshy doctor` 显示 daemon 未运行。

### 修复

```bash
codesign --force --deep -s - ~/.local/bin/arshy ~/.local/bin/arshyd
arshy daemon restart
```

**最容易复发**的场景：手工 `cargo build --release && cp target/release/{arshy,arshyd} ~/.local/bin/` 之后忘记重新签名。安装脚本在每次安装/更新后都会签名；手工替换二进制时请重复这一步。`self-update` 只复制文件、不签名（见《管理 arshy daemon》）。

## 4. 源码改动"不生效"：cargo 缓存假象

改了源码、`cargo build` 也成功，但行为还是旧的——常见于：

- `cargo build` 命中增量缓存，二进制没真正更新（尤其是改了 `parsers/builtin/*.toml`——它们被 `rust-embed` 编译进二进制，**只改 TOML 不触发重编**时尤其隐蔽）；
- daemon 仍在运行旧二进制（daemon 启动时把 parser 和配置加载进内存，`parser reload` 只重载文件系统 parser，**builtin parser 是嵌入的，必须重启 daemon 才更新**）。

### 修复步骤

```bash
# 1) 强制重建 arshy 包（清掉该包的增量缓存）
cargo clean -p arshy
cargo build

# 2) 确认二进制版本/时间戳
./target/debug/arshy --version
ls -l target/debug/arshyd

# 3) 重启 daemon 加载新二进制与新嵌入 parser
./target/debug/arshy daemon restart

# 4) 验证
./target/debug/arshy parser reload
```

对应地，`arshy self-update` 之后也必须 `arshy daemon restart`（命令自身会提示）。检查 daemon 实际运行的二进制：

```bash
ps -o command= -p "$(cat ~/.local/share/arshy/arshyd.pid)"
```

## 5. socket 残留

### 正常清理机制

- daemon 启动时 `lifecycle::cleanup_stale_socket` 会尝试连接 socket：连不上（进程已死）就删除；
- `connect_or_start` 发现 socket 存在但连接失败时，先删 socket 再拉起；
- 优雅关闭时 daemon 自己删除 socket 与 PID 文件。

### 手动清理

daemon 未运行、但文件仍在：

```bash
ls -l ~/.local/share/arshy/arshyd.sock ~/.local/share/arshy/arshyd.pid
rm -f ~/.local/share/arshy/arshyd.sock ~/.local/share/arshy/arshyd.pid
```

> 数据目录里若出现 `arshyd.sock.stale` / `arshyd.pid.stale`，是早期版本/外部工具留下的残留（当前代码不会创建带 `.stale` 后缀的文件）。确认 daemon 未运行后可直接删除，不影响正常使用。

## 6. spawn-lock 残留：自动拉起静默失败

### 原理

`start_daemon` 用原子文件 `/tmp/arshyd.spawn-lock` 防并发拉起：创建成功才 spawn，失败则**静默跳过**（认为另一个进程正在拉）。如果某个 proxy 在"创建锁 → 真正 spawn"之间崩溃，锁文件会残留，之后所有自动拉起都会静默 no-op，表现为"连不上但也不报错"。

### 自动恢复

- `arshy doctor` 检测到锁存在且 daemon 未运行时，会**自动删除**并提示 `A stale spawn-lock was blocking auto-start. It has been removed.`；
- daemon 自身启动时也会清理该文件（`src/daemon/main.rs`：`cleaned up stale spawn-lock`）。

### 手动恢复

```bash
ls -l /tmp/arshyd.spawn-lock
rm -f /tmp/arshyd.spawn-lock
arshy daemon restart
```

## 7. 通用诊断：arshy doctor

```bash
arshy doctor          # 全部检查
arshy doctor --agent codex   # 只看单个 agent
```

`doctor` 检查并给出修复建议：`arshy`/`arshyd` 是否在 PATH、daemon 是否运行（含 spawn-lock 清理）、MCP 注册、Claude 权限（`mcp__arshy__arshy_exec`、`mcp__arshy__arshy_query`、`Bash(arshy *)`、`Bash(arshyd *)`）、macOS TCC 文件访问、shell hook/工作区 opt-in/auto_start 是否齐备、各 agent 集成状态。末尾打印 `N passed, 0 warnings, M failed`，并建议 `arshy install` 自动修复多数问题。

## 8. macOS TCC：命令访问不了 ~/Documents 等目录

daemon 通过 PTY 启动子进程，macOS TCC 可能限制子进程访问 Documents/Desktop/Downloads。`doctor` 第 5 节会探测这些目录。

内置缓解：executor 检测到受限 cwd 时，在 `/tmp/.arshy-cwd/<hash>` 创建符号链接绕过 TCC，并注入 `ARSHY_CWD` 让命令知道真实路径（`src/daemon/exec/cwd.rs` 的 `prepare_cwd`；daemon 启动时会清理上一次会话的残留 symlink）。

永久修复：系统设置 → 隐私与安全性 → 完全磁盘访问权限 → 把你的终端应用加入白名单（`doctor` 的提示原文）。

## 9. 其他高频现象

| 现象 | 处理 |
|---|---|
| `arshy daemon start` 输出 `daemon is already running` 但客户端连不上 | 检查 socket 路径一致性（`arshy config get daemon.socket_path`）；确认没有多个配置来源覆盖了 socket |
| `arshy run "cmd"` 返回 `{"error": {...}}` JSON | 命令被安全过滤拦截（message 含 `command blocked`）或执行失败（`exit_code`）；见《配置与使用安全能力》 |
| `arshy parser list` 显示 `Loaded parsers: 0` | 当前版本该方法读取 daemon status 中的 `parser_count`，而 status 响应不含该字段（实测恒为 0）；用 `arshy parser reload` 看 diff 验证 parser 加载 |
| `arshy daemon stop` 后任务还在跑 | 优雅关闭有 5 秒自然等待 + 30 秒硬截止，期间先尝试 `kill_all`；等待后仍强制退出 |
| 卸载后 `arshy` 命令还在 | 卸载只删除 `~/.local/bin` 下的二进制；PATH 里其他位置的副本（如 `cargo install` 的 `~/.cargo/bin`）需自行删除 |
| 提示 `daemon already running (pid N)` | 另一个 arshyd 实例已存在；不需要手工杀进程，用 `arshy daemon restart` 走正常生命周期 |
