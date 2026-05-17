# 安全

## 默认安全策略

零配置下 arshy 已启用多层防护：

- **命令黑名单**：拦截 `rm -rf /`、`curl|sh`、`dd`、`mkfs`、fork bomb 等危险命令
- **权限分级**：`full`（默认）/ `read-only`
- **路径沙箱**：限制 cwd 白名单
- **审计日志**：所有命令执行记录到 JSON Lines 文件
- **Socket 权限**：Unix Domain Socket 权限 `0600`（owner-only）
- **ReDoS 防护**：Parser 正则编译时静态校验，拒绝嵌套量词和重叠交替
- **沙箱模式校验**：不支持的 sandbox_mode 启动时明确拒绝（不静默降级）

## 配置

```toml
[security]
# 权限级别："full" 或 "read-only"
access_level = "full"

# 命令白名单（设置后仅允许列表中的命令；null = 黑名单模式）
allowed_commands = ["ls", "git", "cargo", "npm", "python3"]

# 额外拦截的正则模式（追加到默认黑名单）
blocked_patterns = ["curl.*\\|.*sh", "eval"]

# 允许的工作目录白名单
sandbox_paths = ["/home/user/projects/"]

# 沙箱模式（当前仅支持 "none"；未来支持 "process" / "container"）
sandbox_mode = "none"

# 审计日志路径（null = 不启用）
audit_log = "/var/log/arshy/audit.log"
```

## 命令过滤

`CommandFilter::from_config()` 在 daemon 启动时编译黑名单正则。**无效正则不会 panic** — 返回 `Err` 并记录 error 日志，回退到宽松模式。启动后所有命令执行前经过 `check()` 方法。

## 审计日志

每行一个 JSON 对象，记录：

```json
{"timestamp":"2026-05-17T12:00:00Z","task_id":"abc-123","command":"cargo build","cwd":"/project","exit_code":0,"blocked":false}
```

写失败（磁盘满、权限变更）不会导致 daemon crash — `AuditLog::log()` 返回 `Err` 并优雅降级。

## 沙箱模式

| 模式 | 状态 | 说明 |
|------|:----:|------|
| `none` | ✅ 已实现 | 不隔离，依赖命令过滤和路径限制 |
| `process` | 🔮 预留 | 进程级隔离（seccomp/pledge） |
| `container` | 🔮 预留 | 容器级隔离 |

配置 `sandbox_mode = "process"` 时 daemon 启动明确拒绝并报错，不会静默降级。

## Socket 安全

Daemon 绑定的 Unix Domain Socket 自动设置权限 `0o600`（仅 owner 可读写），防止同一主机上的其他用户通过 socket 执行命令。
