# 配置参考

> 本文档依据 `src/config/schema.rs`、`src/config/mod.rs`、`src/config/merge.rs` 与 `./target/debug/arshy config list` 实际输出核对（arshy 0.2.0）。

## 加载优先级与来源

合并优先级（高者覆盖低者，`src/config/mod.rs` `Config::load`）：

1. CLI 覆盖（`--log-level`、`--config`；`CliOverrides` 中的 `socket_path`/`store_dir` 字段当前无 CLI 入口暴露）
2. 环境变量（`ARSHY_<SECTION>_<KEY>`）
3. 配置文件（TOML，部分字段覆盖；缺省字段保留低层值）
4. 内置默认值

默认配置文件路径：`~/.config/arshy/config.toml`（`XDG_CONFIG_HOME` 设置时为其下 `arshy/config.toml`）。

路径字段支持占位符展开（`expand_path`）：`${XDG_DATA_HOME}`、`${XDG_CONFIG_HOME}`、`${XDG_CACHE_HOME}`、`${HOME}`；未设置对应 XDG 变量时分别默认 `~/.local/share`、`~/.config`、`~/.cache`。

加载后的保护性钳制（`Config::load`）：

- `daemon.kill_force_ms`、`daemon.kill_graceful_ms` 最小 100ms；
- `daemon.max_task_duration_ms` 最小 1_000ms；
- `store.prune_older_than_days` 最小 1 天。

配置格式版本：`version` 字段。当前 schema 版本为 1（`Config::default()` 即返回 1）；`version > 1` 时启动打印 warning。

## 配置键总表

`arshy config get <key>` 的有效键（`VALID_KEYS`，`src/cli/mod.rs`）：

```
daemon.socket_path
daemon.log_level
daemon.log_format
daemon.auto_start
daemon.max_task_duration_ms
daemon.max_output_bytes
daemon.kill_graceful_ms
daemon.kill_force_ms
daemon.max_concurrent_tasks
daemon.idle_timeout_secs
store.store_dir
store.integrity_check
store.auto_prune
store.prune_keep
store.prune_older_than_days
parser.dirs
parser.hot_reload
notifications.batch_interval_ms
notifications.max_batch_events
```

### daemon 段

| 键 | 类型 | 默认值 | 环境变量 | 说明 |
|---|---|---|---|---|
| `daemon.socket_path` | path | `${XDG_DATA_HOME}/arshy/arshyd.sock` | `ARSHY_DAEMON_SOCKET_PATH` | daemon UDS socket 路径 |
| `daemon.log_level` | string | `info` | `ARSHY_DAEMON_LOG_LEVEL` | 日志级别：`trace`/`debug`/`info`/`warn`/`error` |
| `daemon.log_format` | string | `text` | `ARSHY_DAEMON_LOG_FORMAT` | 日志格式：`text`（compact）/`json` |
| `daemon.auto_start` | bool | `true` | `ARSHY_DAEMON_AUTO_START` | 连接失败时是否自动启动 daemon；布尔解析接受 `true`/`True`/`TRUE`/`1` |
| `daemon.max_task_duration_ms` | u64 | `3600000`（1h） | — | 单任务最大时长（executor 强制超时） |
| `daemon.max_output_bytes` | u64 | `10485760`（10 MiB） | — | 单任务最大输出字节数 |
| `daemon.kill_graceful_ms` | u64 | `3000` | — | kill 升级流程：SIGINT 后等待退出的时长（之后发 SIGTERM） |
| `daemon.kill_force_ms` | u64 | `2000` | — | kill 升级流程：SIGTERM 后等待退出的时长（之后发 SIGKILL） |
| `daemon.max_concurrent_tasks` | u32 | `4` | `ARSHY_DAEMON_MAX_CONCURRENT_TASKS` | 最大并发任务数 |
| `daemon.idle_timeout_secs` | u64 | `900`（15min） | `ARSHY_DAEMON_IDLE_TIMEOUT_SECS` | 空闲自退出秒数（无运行任务且最近完成任务早于该值）；`0` 禁用；环境变量设置时 `0` 保留、其余最小 30 |
| `daemon.sandbox_mode` | string | `none` | — | daemon 启动校验接受 `none`/`workspace` |

### store 段

存储为 JSONL 文件布局（非 SQLite）：`tasks.jsonl`、`events/<task_id>.jsonl`、`raw/<task_id>.txt`、`versions.json`（`src/daemon/store/`）。legacy 的 `db_path`/`wal_mode`/`backend` 字段已移除。

| 键 | 类型 | 默认值 | 环境变量 | 说明 |
|---|---|---|---|---|
| `store.store_dir` | path | `${XDG_DATA_HOME}/arshy` | `ARSHY_STORE_STORE_DIR` | 任务/事件 JSONL 目录 |
| `store.integrity_check` | bool | `true` | — | 启动时校验 `tasks.jsonl` 可解析性 |
| `store.auto_prune` | bool | `false` | — | 启动时自动按 `prune_older_than_days` 清理 |
| `store.prune_keep` | usize | `1000` | `ARSHY_STORE_PRUNE_KEEP` | prune 保留任务数（daemon `prune` 默认值） |
| `store.prune_older_than_days` | u32 | `30` | — | prune 保留天数 |

### parser 段

| 键 | 类型 | 默认值 | 环境变量 | 说明 |
|---|---|---|---|---|
| `parser.dirs` | path 数组 | `["${HOME}/.arshy/parsers"]` | — | 用户 parser 目录；同名用户 parser 覆盖内置（同优先级下 user 优先） |
| `parser.hot_reload` | bool | `true` | `ARSHY_PARSER_HOT_RELOAD` | 是否启用文件系统热重载 watcher（notify v7） |

### notifications 段

| 键 | 类型 | 默认值 | 环境变量 | 说明 |
|---|---|---|---|---|
| `notifications.batch_interval_ms` | u64 | `100` | — | MCP proxy 通知批量窗口（毫秒） |
| `notifications.max_batch_events` | usize | `50` | — | 批量合并的最大事件数（满则立即 flush） |

### mcp / telemetry 段

均为空结构体（`McpConfig {}`、`TelemetryConfig {}`）——当前没有可配置键。

### security 段

| 键 | 类型 | 默认值 | 环境变量 | 说明 |
|---|---|---|---|---|
| `security.blocked_patterns` | string 数组 | 见下 | — | 命令黑名单正则；命中即拒绝（`command blocked: matches pattern '...'`） |
| `security.allowed_commands` | string 数组 | 无（不启用） | — | 命令白名单（按首词 basename 精确匹配）；黑名单始终优先 |
| `security.sandbox_paths` | string 数组 | `[]` | — | cwd 沙箱路径限制（`check_path`） |
| `security.access_level` | string | `full` | `ARSHY_SECURITY_ACCESS_LEVEL` | `full`/`read-only`；`read-only` 时 `task/run` 与 `task/kill` 返回 ACCESS_DENIED |
| `security.audit_log` | string | 无 | — | 审计日志路径（命中黑名单时记录；支持占位符展开） |
| `security.rate_limit.enabled` | bool | `false` | — | 令牌桶限流开关 |
| `security.rate_limit.max_commands_per_second` | f64 | `10.0` | — | 每秒令牌补充速率 |
| `security.rate_limit.burst` | f64 | `20.0` | — | 桶容量（突发上限） |

默认 `blocked_patterns`（25 条，`config list` 输出核对）：

```
rm\s+-rf\s*(?:--\s*)?["']?[/~]
rm\s+--recursive\s+--force\s*["']?[/~]
rm\s*\$\{IFS\}-rf
dd\s+if=
mkfs\.
mkfs\s
curl.*\|\s*(ba)?sh
wget.*\|\s*(ba)?sh
\|\s*(ba)?sh
\|\s*base64\s+-d\s*\|\s*(ba)?sh
base64\s+-d.*\|\s*(ba)?sh
eval\s+\$\(|eval\s+`
sudo\s+.*rm\s+-[a-zA-Z]*[rR]
sudo\s+.*rm\s+-[a-zA-Z]*[fF]
sudo\s+.*\b(dd|mkfs|fdisk|parted)\b
sudo\s+.*\bchmod\s+(-R\s+)?777\b
sudo\s+su\b
su\s+-
cat\s+.*\.ssh/(id_rsa|id_ed25519|id_dsa|id_ecdsa|authorized_keys)
/proc/self/environ
/proc/\d+/environ
nc\s+-l
ncat\s+-l
chmod\s+(-R\s+)?777
:\(\)\{\s*:\|:&\s*\};:
```

## 配置文件示例

```toml
version = 1

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
idle_timeout_secs = 900
sandbox_mode = "none"

[store]
store_dir = "${XDG_DATA_HOME}/arshy"
integrity_check = true
auto_prune = false
prune_keep = 1000
prune_older_than_days = 30

[parser]
dirs = ["${HOME}/.arshy/parsers"]
hot_reload = true

[notifications]
batch_interval_ms = 100
max_batch_events = 50

[mcp]

[telemetry]

[security]
blocked_patterns = []
allowed_commands = []
sandbox_paths = []
access_level = "full"
audit_log = "${XDG_DATA_HOME}/arshy/audit.log"

[security.rate_limit]
enabled = false
max_commands_per_second = 10.0
burst = 20.0
```

## 与直觉不符的事实（代码核对）

1. 环境变量只覆盖 10 个键（见上表），并非所有配置键都有对应环境变量；`--log-level`/`--config` 之外，`CliOverrides` 的 `socket_path`/`store_dir` 字段在 CLI 中无入口。
2. `security.rate_limit` 触发时错误消息为 `rate limit exceeded: too many commands per second`，经 `ArshyError::json_rpc_code()` 映射为 `RATE_LIMITED`（-32005）。
