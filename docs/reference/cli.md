# CLI 参考（arshy / arshyd）

> 本文档依据 `src/main.rs` 的 clap 定义与 `./target/debug/arshy --help`、各子命令 `--help` 的实际输出逐项核对（arshy v0.0.1）。

`arshy` 是面向 AI Agent 的结构化 shell 执行层。CLI 提供两类入口：

- 交互式子命令：`arshy run "cmd"` 等，通过 Unix Domain Socket 连接 `arshyd` daemon 执行；
- MCP stdio proxy：`arshy --from-mcp`，将 stdin 的 MCP JSON-RPC 转发为 IPC 请求（供 Claude Code / Cursor 等集成）。

`arshyd` 是后台 daemon，仅支持 `--version` / `-V` 两个启动参数（`src/daemon/main.rs`）。

## 全局用法

```
Usage: arshy [OPTIONS] [COMMAND]
```

### 全局选项

| 选项 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--from-mcp` | bool | `false` | 以 MCP stdio proxy 模式运行（连接 daemon、转发工具调用、返回结构化结果） |
| `--config <CONFIG>` | path | 无（使用默认路径） | 配置文件路径；未指定时使用默认值 + 环境变量覆盖 |
| `--log-level <LOG_LEVEL>` | string | 无 | 日志级别（`trace`/`debug`/`info`/`warn`/`error`），覆盖配置文件 |
| `-h, --help` | — | — | 打印帮助 |
| `-V, --version` | — | — | 打印版本（0.0.1） |

无子命令时打印一行用法说明并退出（`src/cli/mod.rs` dispatch 的 `None` 分支）。

### 子命令一览

| 子命令 | 用途 |
|---|---|
| `run` | 执行 shell 命令 |
| `list` | 列出任务 |
| `query` | 查询任务事件 |
| `kill` | 终止运行中的任务 |
| `tail` | 查看任务输出 |
| `install` | 注册为 MCP server（Claude Code / Cursor，已弃用，见 `setup`） |
| `uninstall` | 移除 arshy 注册（全部 agent，或 `--agent` 指定单个） |
| `prune` | 清理旧任务历史 |
| `config` | 查看/修改配置（`get`/`list`/`path`） |
| `status` | 显示 daemon 状态 |
| `daemon` | 管理 daemon 进程（`start`/`stop`/`restart`） |
| `stats` | 显示聚合统计 |
| `doctor` | 诊断集成并给出修复建议 |
| `analyze` | 生成影响分析报告（隐藏命令，`dogfood --report` 内部使用） |
| `parser` | 管理 parsers（`reload`/`list`/`benchmark`） |
| `hook` | 管理 shell 集成 hook（`install`/`uninstall`） |
| `init` | 在当前工作区初始化 arshy（`.arshy.toml`、`.mcp.json`、AGENTS.md 指令块） |
| `setup` | 将 arshy 接入 agent 环境（一个命令一个 agent） |
| `self-update` | 将当前构建复制覆盖已安装的 arshy/arshyd |
| `claude-hook` | Claude Code PreToolUse hook 处理器（读 stdin、写 stdout） |

## run

执行一条 shell 命令。自动启动 daemon（`connect_or_start`），发送 IPC `task/run`。

```
Usage: arshy run [OPTIONS] <COMMAND>
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `<COMMAND>` | string | 必填 | 要执行的命令 |
| `--cwd <CWD>` | string | 无 | 工作目录（一次性覆盖；session 级用 `action:cd`） |
| `--timeout-ms <TIMEOUT_MS>` | u64 | 无 | 超时毫秒数 |
| `--mode <MODE>` | string | `auto`（代码内默认） | `auto`/`sync`/`async`；CLI 不做枚举校验，由 executor 解释 |
| `--format <FORMAT>` | string | `auto` | `pretty`（终端 UI）/ `json`（原始 JSON）/ `auto`（默认：stdout 是终端 → pretty，否则 JSON） |
| `--errors-only` | bool | `false` | 只返回 error 级事件 |
| `--purpose <PURPOSE>` | string | 无 | 用途标签（如 `dogfood`），让 stats 区分测试负载与真实开发；未标记任务计为真实 |

行为（`src/cli/tasks.rs` run_command）：

- `--format pretty`：`render_run_text`（短命令原文 / 长命令摘要 + 根因 + 变更文件 + 事件）输出到 **stderr**；
- `--format json`：将 daemon 返回的 JSON 以 pretty 形式打印到 **stdout**（agent 默认路径）；
- `--format auto`（默认）：stdout 是终端 → 人类可读文本（stderr），否则 → JSON（stdout）；
- 进程以命令的 `exit_code` 退出（`std::process::exit`）。

示例：

```bash
arshy run "echo hello"                       # 短命令：JSON 结果（含 raw_output）
arshy run "cargo test" --format json         # 长命令：结构化结果（events/event_count）
arshy run "ls -la" --format pretty           # 终端 UI 输出（stderr）
arshy run "npm run build" --errors-only      # 只返回 error 事件
arshy run "cargo test" --purpose dogfood     # 标记用途，stats 单独统计
```

## list

列出任务（按 `started_at` 倒序，`src/daemon/store/tasks.rs`）。发送 IPC `task/list`，stdout 输出 JSON 数组。

```
Usage: arshy list [OPTIONS]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--status <STATUS>` | string | 无 | 状态过滤（`running`/`completed`/`failed`/`killed`/`timeout`） |
| `--limit <LIMIT>` | usize | `10` | 返回任务数上限 |

## query

查询一个任务的结构化事件。发送 IPC `task/query`，stdout 输出 `{"events": [...], "total": n}`。

```
Usage: arshy query [OPTIONS] <TASK_ID>
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `<TASK_ID>` | string | 必填 | 任务 ID |
| `--event-type <EVENT_TYPE>` | string | 无 | 事件类型过滤（如 `compile_error`、`lint`） |
| `--severity <SEVERITY>` | string | 无 | 严重级别过滤 |
| `--code <CODE>` | string | 无 | 错误码过滤 |
| `--file <FILE>` | string | 无 | 文件路径过滤（子串匹配 `location.file.contains`） |
| `--limit <LIMIT>` | usize | `20` | 最大返回事件数 |

说明：CLI 层 `TASK_ID` 必填；跨任务搜索（省略 `task_id`）仅通过 MCP/IPC 可用。默认排除 `log` 类型事件（`include_logs=false`）。

## kill

终止运行中的任务。发送 IPC `task/kill`，stdout 输出 `{"task_id": ..., "status": "killed"}`。

```
Usage: arshy kill <TASK_ID>
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `<TASK_ID>` | string | 必填 | 任务 ID |

## tail

查看任务输出。发送 IPC `task/tail`，stdout 输出 `{"task_id": ..., "lines": [...]}`。

```
Usage: arshy tail [OPTIONS] <TASK_ID>
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `<TASK_ID>` | string | 必填 | 任务 ID |
| `--lines <LINES>` | usize | `50` | 返回行数 |
| `--format <FORMAT>` | string | `event` | 输出格式；daemon executor 目前忽略该参数（`_format`，见 `src/daemon/exec/mod.rs`） |

## install

注册为 MCP server（历史入口，仅 Claude Code / Cursor；程序会打印弃用提示改用 `setup`）。执行内容（`src/cli/mod.rs` install）：

- 写入 `~/.claude.json`（顶层 `mcpServers.arshy`，`stdio` + `--from-mcp`）；
- 写入 `~/.cursor/mcp.json`；
- 写入 `~/.gemini/config/mcp_config.json`（Antigravity）；
- 合并权限到 `~/.claude/settings.json` 的 allow 列表（`ARSHY_PERMISSIONS`：`mcp__arshy__arshy_exec`、`mcp__arshy__arshy_query`、`Bash(arshy *)`、`Bash(arshyd *)`）；
- 未运行时自动启动 daemon（最多等待 3s）。

```
Usage: arshy install
```

## uninstall

移除 arshy 注册（全部 agent，或 `--agent` 指定单个；单个 agent 走集成模块 `integrate::uninstall_agent`，零残留恢复其配置）。

```
Usage: arshy uninstall [OPTIONS]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--agent <AGENT>` | string | 无 | 只从该 agent 移除（如 `codex`、`claude-code`） |

无 `--agent` 时执行完整卸载：移除 `~/.claude.json`、`~/.cursor/mcp.json`、`~/.gemini/config/mcp_config.json` 中的 arshy 条目、`~/.claude/settings.json` 中的权限、GUI PATH 层与各 agent 集成、shell shim（`~/.arshy/bin` + rc 文件）；若运行于 `~/.local/bin` 则同时删除两个二进制。任务/事件数据保留在 `${XDG_DATA_HOME}/arshy`（提示手动清理）。

## prune

清理旧任务历史。发送 IPC `daemon/prune`，stdout 输出 `{"tasks_deleted": n, "events_deleted": n}`。

```
Usage: arshy prune [OPTIONS]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--keep <KEEP>` | usize | 无 | 仅保留最近 N 个任务（`prune_keep`） |
| `--older-than <OLDER_THAN>` | string | 无 | 删除早于 N 天的任务（解析为 u64 天，`prune_older_than`） |

两者都未提供时 daemon 按 `keep=1000` 处理（`src/daemon/ipc_handler.rs`）。

## config

查看/修改配置。不连接 daemon（纯本地读取）。

```
Usage: arshy config <COMMAND>
```

| 子命令 | 说明 |
|---|---|
| `get <KEY>` | 按点分键取值（如 `daemon.log_level`）；字符串值原样打印，其余 pretty JSON；未知键报错并列出有效键，退出码 1 |
| `list` | 以 TOML 打印合并后的完整配置（默认值 + 文件 + 环境变量 + CLI 覆盖） |
| `path` | 打印配置文件路径（`--config` 指定时打印该路径；否则 `~/.config/arshy/config.toml`） |

示例：

```bash
arshy config get daemon.log_level      # info
arshy config list                      # 完整配置 TOML
arshy config path                      # /Users/<user>/.config/arshy/config.toml
```

## status

显示 daemon 状态。发送 IPC `daemon/status`，stdout 输出 JSON：`uptime_secs`、`tasks_running`、`tasks_total`、`db_size_bytes`、`counters`（`connections_accepted`、`events_emitted`、`tasks_created`、`tasks_completed`、`tasks_failed`）。

```
Usage: arshy status
```

## daemon

管理 daemon 进程。

```
Usage: arshy daemon <COMMAND>
```

| 子命令 | 行为 |
|---|---|
| `start` | 若 socket 可连接则提示"already running"；否则经 `start_daemon()` 分离式 spawn（sibling `arshyd` + `setsid` + spawn-lock + 熔断），最多等待 5s（25 × 200ms）确认 |
| `stop` | 发送 IPC `daemon/shutdown`，stdout 输出 `{"status": "shutting_down"}` |
| `restart` | 先 stop（忽略未运行错误），等待 500ms，再 start |

## stats

显示聚合统计。发送 IPC `daemon/stats`。

```
Usage: arshy stats [OPTIONS]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--format <FORMAT>` | string | `pretty` | `pretty`（终端 UI，输出到 stderr）/ `json`（原始 JSON，输出到 stdout） |

JSON 字段（`StatsResponse`，`src/ipc/mod.rs`）：`total_tasks`、`by_status`（`running`/`completed`/`failed`/`killed`/`timeout` 计数）、`total_events`、`total_errors`、`purpose_breakdown`、`avg_duration_ms`、`p50_duration_ms`、`p99_duration_ms`、`failure_rate`、`db_size_bytes`、`parser_coverage_pct`、`context_enriched`、`dedup_collapsed`、`correlated_errors`、`per_parser_usage`、`total_raw_output_bytes`、`total_agent_delivered_bytes`、`total_agent_visible_events`、`total_agent_skipped_events`、`total_locations_extracted`、`total_codes_extracted`、`total_contexts_enriched`。

## doctor

诊断集成并显示修复建议。分为：1. Binaries（PATH 中 arshy/arshyd）、2. Daemon（socket、stale spawn-lock 清理）、3. MCP server config（`~/.claude.json`、`~/.cursor/mcp.json`）、4. Permissions（`~/.claude/settings.json`）、5. Filesystem access（macOS TCC 探测）、5.5 Agent shell interception（hook shim、workspace 标记、auto-start）、5.6 Agent integrations。

```
Usage: arshy doctor [OPTIONS]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--agent <AGENT>` | string | 无 | 只检查该 agent；合法值：`codex`、`claude-code`、`cursor`、`vscode`、`antigravity`、`opencode`、`aider`、`workbuddy`；未知值直接报错 |

## analyze

生成影响分析报告（隐藏命令，`#[command(hide = true)]`，供 `dogfood --report` 内部使用）。发送 IPC `daemon/analyze`。

```
Usage: arshy analyze [OPTIONS]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--format <FORMAT>` | string | `auto` | `pretty`（终端 UI，stderr）/ `json`（原始 JSON，stdout）/ `auto`（默认） |

报告结构（`src/daemon/analytics.rs` `ImpactReport`）：`summary`、`token_efficiency`、`information_density`、`command_patterns`、`repair_loop`、`temporal`、`generated_at`。

## parser

管理 parsers。

```
Usage: arshy parser <COMMAND>
```

| 子命令 | 行为 |
|---|---|
| `reload` | 发送 IPC `parser/reload`，在 stderr 打印 diff（+/- 着色）；diff 为空字符串时打印 "no changes" |
| `list` | 发送 IPC `daemon/status` 并读取响应中的 `parser_count` 字段打印"Loaded parsers: N" |
| `benchmark` | 运行 `cargo test --lib daemon::parser::benchmark::run_benchmark -- --nocapture`，截取 `=== ARSHY BENCHMARK ===` 与 `=== END BENCHMARK ===` 之间的 JSON，写入 `docs/benchmark_report.json`（存在 docs 目录时）或 `.arshy-benchmark.json`，并与旧基线对比输出渲染报告 |

## hook

管理 shell 集成 hook。

```
Usage: arshy hook <COMMAND>
```

| 子命令 | 行为 |
|---|---|
| `install` | 创建 `~/.arshy/bin/{sh,bash,zsh,claude-hook}` 指向 arshy 的 symlink；在 `~/.zshrc`/`~/.bashrc` 添加 PATH 前缀（幂等，标记 `# Arshy shell integration hook`）；注册 Claude Code PreToolUse hook |
| `uninstall` | 移除 symlink（空目录一并删除）、rc 行、Claude Code hook |

## init

在当前工作区初始化 arshy（三层，`src/cli/shell_wrapper.rs` init_workspace）：

1. `.arshy.toml` — bash-proxy opt-in 标记（存在该文件或 `.arshy/` 目录即拦截 `bash -c`）；
2. `.mcp.json` — 项目级 MCP 注册，任何 MCP-capable agent 可发现；
3. `AGENTS.md` — arshy 指令块（路由 shell 命令经 arshy）。

```
Usage: arshy init [OPTIONS]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--undo` | bool | `false` | 撤销 init：移除 `.arshy.toml` 标记、`.mcp.json` 条目与 `AGENTS.md` 区块（项目层零残留） |

## setup

将 arshy 接入 agent 环境（每个 agent 一条命令）。

```
Usage: arshy setup [OPTIONS] [AGENT]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `[AGENT]` | string | 无（全部已检测 agent） | agent id：`codex`、`claude-code`、`cursor`、`vscode`、`antigravity`、`opencode`、`aider`、`workbuddy` |
| `--dry-run` | bool | `false` | 只显示将要执行的操作，不做改动 |
| `--status` | bool | `false` | 只打印各 agent 集成状态（不做改动） |

## self-update

将当前构建复制覆盖已安装的 arshy/arshyd 二进制（以及同目录 sibling `arshyd`）。

```
Usage: arshy self-update [OPTIONS]
```

| 参数 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `--dest <DEST>` | path | 见说明 | 安装目录；默认：开发构建（路径含 `target`）→ `~/.local/bin`；已安装 → 当前二进制所在目录（no-op） |

复制后显式设置 0o755 权限，并提示 `arshy daemon restart`。

## claude-hook

Claude Code PreToolUse hook 处理器：从 stdin 读取 JSON（`tool_name` + `tool_input.command`），将 `Bash` 工具调用改写为 `ARSHY_BYPASS=1 arshy run '<cmd>'` 并输出 `hookSpecificOutput.permissionDecision=allow`；arshy/arshyd/claude-hook 自身调用不改写。

```
Usage: arshy claude-hook
```

## 退出码

- `run`：以被执行命令的 `exit_code` 退出；
- 其余命令：成功 0；`config get` 未知键、`doctor --agent` 未知 agent 等错误返回非零并打印错误信息。

## 与直觉不符的事实（代码核对）

1. `arshy run --format auto` 与 `--format json` 行为相同（匹配分支只有 `pretty` 与其他）。
2. `install` 在**运行时**打印弃用提示（改用 `setup`），但 clap 的 `--help` 输出并未标记 deprecated，也不隐藏。
3. `analyze` 为隐藏命令（`hide = true`），不出现在 `arshy --help` 的命令列表中。
