# 安全

## 默认安全策略

零配置下 arshy 已启用基本防护：

- **命令黑名单**：拦截 `rm -rf /`、`curl|sh`、`dd`、`mkfs`、fork bomb 等
- **权限分级**：`full`（默认）/ `read-only`
- **路径沙箱**：可选限制 cwd 范围

## 配置

```toml
[security]
# 权限级别："full" 或 "read-only"
access_level = "full"

# 命令白名单（设置后仅允许列表中的命令）
allowed_commands = null  # null = 使用黑名单模式

# 自定义黑名单（追加到默认黑名单）
blocked_patterns = [
    "rm\\s+-rf\\s+/",
    "curl.*\\|\\s*sh",
]

# 路径沙箱（限制 cwd 范围）
sandbox_paths = []

# 审计日志路径（null = 不记录）
audit_log = "~/.local/share/arshy/audit.log"
```

## 命令过滤

两种模式：

### 黑名单模式（默认）

`allowed_commands = null`。默认拦截：

| 模式 | 说明 |
|------|------|
| `rm\s+-rf\s+/` | 根目录删除 |
| `rm\s+-rf\s+~/` | 主目录删除 |
| `curl.*\|\s*sh` | 远程代码执行 |
| `wget.*\|\s*sh` | 远程代码执行 |
| `dd\s+if=` | 磁盘覆写 |
| `mkfs` | 文件系统格式化 |
| `:(){ :|:& };:` | fork bomb |

可追加自定义 `blocked_patterns`。

### 白名单模式

设置 `allowed_commands` 后仅允许列表中的命令前缀：

```toml
[security]
allowed_commands = ["git", "cargo", "npm", "ls", "cat"]
```

`cargo build` ✅（前缀匹配），`rm -rf /` ❌。

## 路径沙箱

限制命令的 `cwd` 必须在指定路径下：

```toml
[security]
sandbox_paths = ["/home/user/projects", "/tmp"]
```

`cwd = "/etc"` ❌，`cwd = "/home/user/projects/app"` ✅。

## 权限分级

| 级别 | run | kill | list/query | stats |
|------|-----|------|------------|-------|
| `full` | ✅ | ✅ | ✅ | ✅ |
| `read-only` | ❌ | ❌ | ✅ | ✅ |

`read-only` 模式下 run/kill 返回 `ACCESS_DENIED (-32003)`。

## 审计日志

启用后记录所有命令执行和拦截事件：

```toml
[security]
audit_log = "~/.local/share/arshy/audit.log"
```

格式：JSON Lines，每行包含时间戳、命令、结果（allowed/blocked/failed）。

## MCP 端安全

Agent 通过 MCP 执行的命令同样受安全策略约束。安全过滤在 daemon 侧强制执行，proxy 无法绕过。
