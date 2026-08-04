# 配置 arshy

本文解决以下任务：定位配置文件、理解配置来源与覆盖优先级、用环境变量和 CLI 参数覆盖配置、按需修改常用配置项、验证配置是否生效。

配置系统的实现在 `src/config/`（`mod.rs` / `merge.rs` / `schema.rs`）。合并顺序（高优先级覆盖低优先级）为：

```
CLI 参数 > 环境变量 > 配置文件 > 内置默认值
```

## 1. 查看当前生效配置

### 查看配置文件路径

```bash
arshy config path
```

默认输出（macOS/Linux，未设置 `XDG_CONFIG_HOME` 时）：

```
/Users/<you>/.config/arshy/config.toml
```

默认路径规则（`src/config/mod.rs` 的 `default_config_path`）：

- 配置文件：`$XDG_CONFIG_HOME/arshy/config.toml`，`XDG_CONFIG_HOME` 未设置时取 `~/.config`。
- 数据目录（任务/事件 JSONL、PID、socket）：`$XDG_DATA_HOME/arshy`，默认 `~/.local/share/arshy`。
- 缓存目录：`$XDG_CACHE_HOME`，默认 `~/.cache`（当前构建中未写入缓存文件，但路径展开支持该变量）。

### 查看完整生效配置

```bash
arshy config list
```

输出为合并后的 TOML（默认值 + 文件 + 环境变量 + CLI 覆盖全部生效后的结果）。`config list` 是唯一能一次性确认所有来源合并结果的命令。典型输出开头：

```toml
version = 0

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
```

> 说明：`version` 是配置 schema 版本字段，当前 schema 版本为 1（`src/config/schema.rs` 中 `default_config_version`）。`config list` 的序列化路径对未写文件的默认值输出 `0`；只有当配置文件里显式写了更高版本时，加载才会对 `version > 1` 打印警告。`version` 无需手工配置。

### 查看单个配置项

```bash
arshy config get daemon.log_level      # 输出: info
arshy config get store.store_dir       # 输出: ${XDG_DATA_HOME}/arshy
arshy config get security.access_level # 输出: full
```

`config get` 使用点号路径导航，不存在的键会报错并列出合法键：

```
unknown key: daemon.xxx
valid keys: daemon.socket_path, daemon.log_level, daemon.log_format, ...
```

合法键列表（`src/cli/mod.rs` 的 `VALID_KEYS`）：

```
daemon.socket_path          daemon.log_level           daemon.log_format
daemon.auto_start           daemon.max_task_duration_ms daemon.max_output_bytes
daemon.kill_graceful_ms     daemon.kill_force_ms       daemon.max_concurrent_tasks
daemon.idle_timeout_secs    store.store_dir            store.integrity_check
store.auto_prune            store.prune_keep           store.prune_older_than_days
parser.dirs                 parser.hot_reload          notifications.batch_interval_ms
notifications.max_batch_events  mcp.client_detection_order
```

## 2. 创建/修改配置文件

配置文件是可选文件：不存在时全部使用默认值。文件采用 TOML 格式，且是**部分覆盖**（partial）语义——每个 section 的每个字段都可省略，只写需要覆盖的字段（`src/config/schema.rs` 的 `PartialConfig` 系列类型）。

```toml
# ~/.config/arshy/config.toml
version = 1

[daemon]
log_level = "debug"
idle_timeout_secs = 0        # 0 = 禁用空闲自动退出

[store]
auto_prune = true
prune_keep = 500

[parser]
dirs = ["${HOME}/.arshy/parsers", "${HOME}/work/parsers"]
hot_reload = true
```

加载流程（`Config::load`，`src/config/mod.rs`）：

1. 从默认值开始；
2. 读入配置文件（`--config` 指定的路径，或默认路径），只覆盖写了的字段；
3. 应用环境变量；
4. 应用 CLI 覆盖；
5. 检查 `version > 1` 时打印"配置来自更新版本"警告；
6. 做安全下限钳制（见第 5 节）。

路径字段支持 `${XDG_DATA_HOME}`、`${XDG_CONFIG_HOME}`、`${XDG_CACHE_HOME}`、`${HOME}` 占位符展开（`expand_path`）。占位符展开发生在 daemon 启动时，因此修改路径后需要重启 daemon 才会生效。

## 3. 用环境变量覆盖配置

环境变量格式为 `ARSHY_<SECTION>_<KEY>`（`src/config/merge.rs` 的 `apply_env`）。布尔值解析：`true`/`True`/`TRUE`/`1` 为真，其余为假。

| 环境变量 | 对应配置键 | 说明 |
|---|---|---|
| `ARSHY_DAEMON_SOCKET_PATH` | `daemon.socket_path` | UDS socket 路径 |
| `ARSHY_DAEMON_LOG_LEVEL` | `daemon.log_level` | `trace`/`debug`/`info`/`warn`/`error` |
| `ARSHY_DAEMON_LOG_FORMAT` | `daemon.log_format` | `text` 或 `json` |
| `ARSHY_DAEMON_AUTO_START` | `daemon.auto_start` | 是否按需自启 daemon |
| `ARSHY_DAEMON_MAX_CONCURRENT_TASKS` | `daemon.max_concurrent_tasks` | 最大并发任务数 |
| `ARSHY_DAEMON_IDLE_TIMEOUT_SECS` | `daemon.idle_timeout_secs` | 空闲退出秒数；`0` 禁用；非 0 时钳制为至少 30 |
| `ARSHY_STORE_STORE_DIR` | `store.store_dir` | JSONL 数据目录 |
| `ARSHY_STORE_PRUNE_KEEP` | `store.prune_keep` | prune 保留条数 |
| `ARSHY_PARSER_HOT_RELOAD` | `parser.hot_reload` | 是否启用 parser 热重载 |
| `ARSHY_SECURITY_ACCESS_LEVEL` | `security.access_level` | `full` 或 `read-only` |

示例：

```bash
export ARSHY_DAEMON_LOG_LEVEL=debug
export ARSHY_DAEMON_AUTO_START=false
export ARSHY_STORE_STORE_DIR=/var/tmp/arshy-store
arshy config list   # 验证：log_level 变为 "debug"
```

此外，日志过滤还受 `RUST_LOG` 影响：`init_logging` 优先使用 `EnvFilter::try_from_default_env()`（即 `RUST_LOG`），未设置时才使用配置的 `daemon.log_level`。

## 4. 用 CLI 参数覆盖配置

顶层参数（`src/main.rs`）：

```bash
arshy --config /path/to/config.toml run "cargo build"
arshy --log-level trace status
```

- `--config <path>`：指定配置文件路径，覆盖默认路径（`config path` 也会随之改变）。
- `--log-level <level>`：覆盖配置文件与环境变量中的 `daemon.log_level`。

`--log-level` 优先级最高（`apply_cli` 在 `apply_env` 之后执行）。注意：CLI 参数是**进程级**覆盖，只影响该次命令；要持久生效请写入配置文件。

## 5. 常用配置项示例

### 5.1 daemon 调优

```toml
[daemon]
# socket 与日志
socket_path = "/tmp/arshy/arshyd.sock"     # 覆盖默认的 ${XDG_DATA_HOME}/arshy/arshyd.sock
log_level = "info"                         # trace|debug|info|warn|error
log_format = "json"                        # text|json

# 任务执行
max_task_duration_ms = 3600000             # 单任务上限，默认 1 小时
max_output_bytes = 10485760                # 输出上限，默认 10 MiB
max_concurrent_tasks = 4                   # 默认 4
kill_graceful_ms = 3000                    # 优雅终止等待（SIGINT/SIGTERM 阶段）
kill_force_ms = 2000                       # 强制终止等待（SIGKILL 阶段）

# 生命周期
auto_start = true                          # 首次命令按需自启 daemon（默认 true）
idle_timeout_secs = 900                    # 空闲 15 分钟自退；0 = 禁用

# 沙箱（daemon 启动时校验，只接受 "none" 或 "workspace"）
sandbox_mode = "none"
```

修改 `socket_path`/`store_dir`/`sandbox_mode` 后必须重启 daemon：`arshy daemon restart`。

### 5.2 store：JSONL 数据与自动清理

```toml
[store]
store_dir = "${XDG_DATA_HOME}/arshy"   # tasks.jsonl、events/<id>.jsonl、raw/<id>.txt、versions.json
integrity_check = true                 # 启动时完整性检查
auto_prune = true                      # 启动时自动清理旧任务
prune_keep = 1000                      # 保留最近 1000 条
prune_older_than_days = 30             # 超过 30 天（钳制下限 1 天）
```

### 5.3 parser：自定义 parser 目录

```toml
[parser]
dirs = ["${HOME}/.arshy/parsers", "/opt/my-org/arshy-parsers"]
hot_reload = true
```

- 目录里的 `.toml` 文件会在 daemon 启动时加载，优先级高于同名 builtin parser（见《创建自定义 parser》）。
- `hot_reload = true`（默认）时，daemon 用 `notify` 监视这些目录，`.toml` 文件增删改会触发热重载；也可随时用 `arshy parser reload` 手动重载。

### 5.4 notifications：事件推送批量

```toml
[notifications]
batch_interval_ms = 100    # 通知批量窗口，默认 100ms
max_batch_events = 50      # 单批最多事件，默认 50
```

### 5.5 security：安全能力

```toml
[security]
blocked_patterns = [ ... ]   # 覆盖默认拦截正则列表（默认 25 条，见《配置与使用安全能力》）
allowed_commands = ["ls", "git", "cargo"]   # 设置后启用白名单模式（未列出的一律拦截）
sandbox_paths = ["/Users/me/projects"]      # 工作目录沙箱
access_level = "full"                       # "full" | "read-only"（read-only 禁止 run/kill）
audit_log = "${XDG_DATA_HOME}/arshy/audit.jsonl"  # 审计日志路径

[security.rate_limit]
enabled = true
max_commands_per_second = 10.0   # 令牌补充速率
burst = 20.0                     # 令牌桶容量
```

## 6. 验证配置生效

配置是 daemon 启动时加载的，修改文件后按顺序验证：

```bash
# 1) 确认合并结果
arshy config list

# 2) 确认单个键
arshy config get daemon.log_level

# 3) 重启 daemon 使新配置生效（socket/store/parser 目录类变更必须重启）
arshy daemon restart
```

## 7. 需要注意的边界行为

- **非法 log level**：`init_logging` 不认识的值会回退到 `info` 并向 stderr 打印 `unknown log level '...'`。
- **安全钳制**（`Config::load` 第 6 步）：`kill_force_ms`/`kill_graceful_ms` 至少 100ms，`max_task_duration_ms` 至少 1000ms，`prune_older_than_days` 至少 1 天。
- **`sandbox_mode` 校验**：daemon 启动时只接受 `"none"` 或 `"workspace"`，其他值导致启动失败（`Config` 错误）。`"workspace"` 会把 daemon 启动时的工作目录加入 `sandbox_paths`。
- **配置错误会阻止启动**：配置文件 TOML 语法错误时，`arshy`/`arshyd` 会直接报错退出；`security.blocked_patterns` 中非法正则的后果见《配置与使用安全能力》。
