# 故障排查

## daemon 无法启动

**"daemon already running (pid xxx)"**

已有 daemon 实例在运行。检查：

```bash
pgrep arshyd
arshy status
```

如需重启：

```bash
arshy daemon restart
```

**socket 未出现**

```bash
# 检查日志
cat /tmp/arshyd.log

# 手动清理 stale socket 和 PID
rm -f ~/.local/share/arshy/arshyd.sock ~/.local/share/arshy/arshyd.pid
```

**"circuit breaker tripped"**

Daemon 在 2 分钟内 crash 超过 5 次，自动重启被抑制。检查日志查找 crash 原因，修复后手动启动：

```bash
arshy daemon start
```

## arshy_exec 返回 DaemonUnreachable

1. 确认 daemon 在运行：`arshy status`
2. 确认 socket 存在：`ls -la ~/.local/share/arshy/arshyd.sock`
3. 确认 socket 权限：应为 `srwx------`（0600）
4. 重启 daemon：`arshy daemon restart`

Proxy 会在连接断开时自动重连。健康检查指数退避防止雷群。

## 命令被拦截

**"command blocked"**

检查安全配置：

```bash
arshy config get security.blocked_patterns
arshy config get security.allowed_commands
```

或在 `~/.config/arshy/config.toml` 中查看。

**"access denied"（read-only 模式）**

检查 `access_level` 配置：

```bash
arshy config get security.access_level
```

## Parser 问题

**输出全是 raw（log 类型）**

1. 确认 parser 被正确检测：长输出命令（build/test）不会走短路径
2. 检查命令首词是否命中 `detect` 匹配（链式命令用分段检测）
3. 查看 daemon 日志中 parser 加载信息

**Parser 修改后未生效**

热重载默认启用。如未生效：
1. 确认文件在 `~/.arshy/parsers/` 目录
2. 检查 TOML 语法：`arshy doctor`
3. 手动触发重载：`pkill -HUP arshyd`（预留）

**自定义 parser 的 pattern 被跳过**

检查 daemon 日志：
- `"ReDoS: nested quantifier detected"` → 正则被拒绝，简化 pattern
- `"invalid regex"` → 修复正则语法

## 测试 parser

```bash
# 生成/更新 fixture
ARSHY_BLESS=1 cargo test --bin arshyd

# 运行 parser 测试
cargo test --bin arshyd fixture
```

## 日志

设置日志级别为 debug 获取详细诊断：

```bash
arshy --log-level debug daemon start
# 或
export ARSHY_DAEMON_LOG_LEVEL=debug
```

日志输出到 stderr（默认 text 格式，可配置 json）。

## 集成诊断

```bash
arshy doctor
```

检查：daemon 状态、socket 权限、MCP 配置、parser 加载、数据库完整性。
