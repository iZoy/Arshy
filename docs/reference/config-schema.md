# 配置 Schema

## [daemon]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `socket_path` | path | `${XDG_DATA_HOME}/arshy/arshyd.sock` | Unix socket 路径 |
| `log_level` | string | `"info"` | 日志级别 |
| `log_format` | string | `"text"` | 日志格式：text / json |
| `auto_start` | bool | `true` | 未运行时自动启动 daemon |
| `max_task_duration_ms` | u64 | `3600000` | 任务超时（1 小时） |
| `max_output_bytes` | u64 | `10485760` | 最大输出量（10 MB） |
| `kill_graceful_ms` | u64 | `3000` | SIGINT 等待时间 |
| `kill_force_ms` | u64 | `2000` | SIGTERM→SIGKILL 等待时间 |
| `max_concurrent_tasks` | u32 | `4` | 最大并发任务数 |
| `sandbox_mode` | string | `"none"` | 沙箱模式（预留） |

## [store]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `db_path` | path | `${XDG_DATA_HOME}/arshy/arshy.db` | SQLite 数据库路径 |
| `wal_mode` | bool | `true` | WAL 模式 |
| `integrity_check` | bool | `true` | 启动时完整性校验 |
| `auto_prune` | bool | `false` | 启动时自动清理 |
| `prune_keep` | usize | `1000` | 保留任务数 |
| `prune_older_than_days` | u32 | `30` | 清理天数阈值 |
| `backend` | string | `"sqlite"` | 存储后端（预留） |

## [parser]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `dirs` | path[] | `["${HOME}/.arshy/parsers"]` | 用户 parser 搜索目录 |
| `hot_reload` | bool | `true` | 文件变更自动重载 |
| `fallback_to_raw` | bool | `true` | 无匹配时回退到 raw |
| `default_priority` | u32 | `50` | 默认优先级 |
| `coverage_warning_threshold` | f64 | `0.3` | 覆盖率告警阈值 |
| `version_cache_ttl_hours` | u64 | `24` | 工具版本缓存 TTL |

## [notifications]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `enabled` | bool | `true` | 启用通知 |
| `batch_interval_ms` | u64 | `100` | 批处理间隔 |
| `max_batch_events` | usize | `50` | 单批最大事件数 |
| `min_severity` | string | `"info"` | 最低通知严重度 |

## [mcp]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `client_detection_order` | string[] | `["claude-code", "cursor", "windsurf"]` | MCP 客户端检测顺序 |

## [security]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `blocked_patterns` | string[] | 7 条默认规则 | 命令黑名单正则 |
| `allowed_commands` | string[]? | `null` | 命令白名单（null = 黑名单模式） |
| `sandbox_paths` | string[] | `[]` | cwd 允许路径 |
| `access_level` | string | `"full"` | 权限级别 |
| `audit_log` | string? | `null` | 审计日志路径 |

## [telemetry]

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `enabled` | bool | `false` | 遥测开关 |

## 完整示例

```toml
[daemon]
socket_path = "${XDG_DATA_HOME}/arshy/arshyd.sock"
log_level = "info"
log_format = "text"
auto_start = true
max_task_duration_ms = 3600000
max_output_bytes = 10485760
kill_graceful_ms = 3000
kill_force_ms = 2000
max_concurrent_tasks = 4

[store]
db_path = "${XDG_DATA_HOME}/arshy/arshy.db"
wal_mode = true
integrity_check = true
auto_prune = false
prune_keep = 1000
prune_older_than_days = 30

[parser]
dirs = ["${HOME}/.arshy/parsers"]
hot_reload = true
fallback_to_raw = true

[notifications]
enabled = true
batch_interval_ms = 100
max_batch_events = 50
min_severity = "info"

[security]
access_level = "full"
audit_log = null
```
