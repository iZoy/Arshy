# 配置 Schema

配置文件位置：`~/.config/arshy/config.toml`。格式版本 `version = 1`。

## [daemon]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `socket_path` | path | `${XDG_DATA_HOME}/arshy/arshyd.sock` | Unix socket 路径 |
| `log_level` | string | `"info"` | 日志级别（非法值 fallback 到 info） |
| `log_format` | string | `"text"` | 日志格式：text / json |
| `auto_start` | bool | `true` | 未运行时自动启动 daemon |
| `max_task_duration_ms` | u64 | `3600000` | 任务最大执行时间（最小 1000ms） |
| `max_output_bytes` | u64 | `10485760` | 任务最大输出字节数（10MB） |
| `kill_graceful_ms` | u64 | `3000` | SIGINT 后等待时间（最小 100ms） |
| `kill_force_ms` | u64 | `2000` | SIGTERM 后等待时间（最小 100ms） |
| `max_concurrent_tasks` | u32 | `4` | 最大并发任务数 |
| `sandbox_mode` | string | `"none"` | 沙箱模式（仅 "none" 可用） |

## [store]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `db_path` | path | `${XDG_DATA_HOME}/arshy/arshy.db` | 数据库路径 |
| `wal_mode` | bool | `true` | WAL 模式（推荐开启） |
| `integrity_check` | bool | `false` | 启动时完整性检查 |
| `auto_prune` | bool | `false` | 自动清理旧数据 |
| `prune_keep` | usize | `10000` | 自动清理时保留的最大任务数 |
| `prune_older_than_days` | u32 | `30` | 自动清理的天数阈值（最小 1 天） |
| `backend` | string | `"sqlite"` | 存储后端（仅 SQLite） |

## [parser]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `dirs` | string[] | `["~/.arshy/parsers"]` | 用户 parser 目录 |
| `hot_reload` | bool | `true` | 文件变更时自动热重载 |
| `fallback_to_raw` | bool | `true` | Parser 匹配失败时回退到 raw |
| `default_priority` | u32 | `50` | 用户 parser 的默认优先级 |
| `coverage_warning_threshold` | f64 | `0.3` | 覆盖率警告阈值（最小 0.01） |
| `version_cache_ttl_hours` | u64 | `24` | 工具版本缓存时效 |

## [notifications]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `enabled` | bool | `true` | 启用 MCP 通知推送 |
| `batch_interval_ms` | u64 | `100` | 批量汇聚窗口（ms） |
| `max_batch_events` | usize | `10` | 批量发送前最大事件数 |
| `min_severity` | string | `"info"` | 最小推送严重度 |

## [mcp]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `client_detection_order` | string[] | `["claude","cursor"]` | 客户端自动检测顺序 |

## [telemetry]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `enabled` | bool | `false` | 启用遥测上报（原子计数器始终启用） |

## [security]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `blocked_patterns` | string[] | 内置黑名单 | 命令拦截正则（编译失败 fallback 到宽松模式） |
| `allowed_commands` | string[] | null | 白名单（设置后禁用黑名单） |
| `sandbox_paths` | string[] | `[]` | 允许的工作目录白名单 |
| `access_level` | string | `"full"` | 权限级别：full / read-only |
| `audit_log` | string | null | 审计日志文件路径（null = 不启用） |

## 环境变量

所有配置项可通过 `ARSHY_<SECTION>_<KEY>` 环境变量覆盖：

```bash
ARSHY_DAEMON_LOG_LEVEL=debug
ARSHY_DAEMON_SOCKET_PATH=/tmp/custom.sock
ARSHY_STORE_DB_PATH=/tmp/test.db
ARSHY_PARSER_HOT_RELOAD=false
ARSHY_SECURITY_ACCESS_LEVEL=read-only
```

## 安全边界（自动 clamp）

| 字段 | 最小值 | 原因 |
|------|:-----:|------|
| `kill_force_ms` | 100 | 防止跳过 force-kill |
| `kill_graceful_ms` | 100 | 防止跳过优雅等待 |
| `max_task_duration_ms` | 1000 | 防止任务立即超时 |
| `prune_older_than_days` | 1 | 防止误删当天数据 |
| `coverage_warning_threshold` | 0.01 | 防止全部行都警告 |
