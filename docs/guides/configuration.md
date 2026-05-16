# 配置

## 配置文件

默认路径：`~/.config/arshy/config.toml`

```bash
arshy config path          # 显示路径
arshy config list          # 列出所有配置
arshy config get daemon.log_level
arshy config set daemon.log_level debug
```

## 分层覆盖

优先级从高到低：**CLI 参数 > 环境变量 > 配置文件 > 默认值**

### 环境变量

格式：`ARSHY_<SECTION>_<KEY>`，全大写，下划线分隔。

```bash
export ARSHY_DAEMON_LOG_LEVEL=debug
export ARSHY_STORE_DB_PATH=/tmp/arshy.db
export ARSHY_SECURITY_ACCESS_LEVEL=read-only
```

### CLI 参数

```bash
arshy --log-level debug --config /path/to/config.toml run "echo hi"
```

## 最小配置

零配置即可运行。以下为推荐的生产配置：

```toml
[daemon]
log_level = "info"
max_task_duration_ms = 600000    # 10 分钟

[store]
auto_prune = true
prune_older_than_days = 7

[security]
access_level = "full"
audit_log = "~/.local/share/arshy/audit.log"

[notifications]
batch_interval_ms = 100
max_batch_events = 50
```

## 路径变量

配置文件中的路径支持变量展开：

| 变量 | 展开为 |
|------|--------|
| `${XDG_DATA_HOME}` | `~/.local/share` |
| `${XDG_CONFIG_HOME}` | `~/.config` |
| `${XDG_CACHE_HOME}` | `~/.cache` |
| `${HOME}` | 用户主目录 |

完整配置项参考：[配置 Schema](../reference/config-schema.md)
