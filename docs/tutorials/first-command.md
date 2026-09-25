# 运行第一条命令

本教程带你跑通 arshy 的第一条命令：短命令的即时返回、长命令的结构化解析、输出格式、任务历史，以及统计与影响分析。完成本教程后，你会知道 `arshy run` 返回的每个字段意味着什么，并能用 `list` / `query` / `tail` / `kill` 管理你的任务。

前置条件：已完成[安装 arshy](install.md)，且 `arshy --version` 能正常输出。

## 第 1 步：跑一条短命令

```bash
arshy run "echo hello from arshy"
```

首次运行会自动启动 daemon（按需自动启动，无需手动 `daemon start`）。命令完成后，终端输出一段 JSON：

```json
{
  "duration_ms": 5,
  "event_count": 0,
  "events": [],
  "exit_code": 0,
  "raw_output": "hello from arshy",
  "short_command": true,
  "status": "completed",
  "task_id": "c795991b-d8f8-4034-8e09-edf89b1a32fd"
}
```

字段含义：

| 字段 | 含义 |
| --- | --- |
| `status` | `completed` / `failed` / `timeout` / `killed` / `running` |
| `exit_code` | 命令退出码；daemon 杀掉的任务为负值 |
| `raw_output` | 原始标准输出 |
| `events` | 结构化事件列表（诊断、location、test_result 等） |
| `short_command` | `true` 表示走了零开销的短命令路径 |
| `task_id` | 任务 ID，后续用 `query` / `tail` / `kill` 引用 |

`echo` 属于短命令白名单：直接返回捕获文本，不做解析，因此 `events` 为空。这里的“捕获文本”经过逐行读取与 UTF-8 展示处理，并不保证保留原始字节。默认输出格式是 JSON——arshy 是 agent-first 的 shell，JSON 是给 agent 消费的默认形态（见第 4 步）。

## 第 2 步：观察自动模式（auto-mode）

`arshy run` 默认 `--mode auto`，自动区分命令长短：

- **原始快速路径**（`ls`、`echo`、`git status` 等检查类工具）：立即返回捕获文本，零解析开销；不创建持久任务记录，因此该 task ID 不能后续查询或重新读取。
- **结构化路径**：命令命中 parser 资产，或者包含 chaining、重定向、后台运行、`--watch` 等生命周期信号时，输出进入 6 层解析管线，事件去重、错误附带 `file:line` 与源代码上下文。新增 parser TOML 不需要再修改 Rust 命令名单。

判断一条命令是否走长路径，看返回里的 `short_command` 字段即可。

## 第 3 步：跑一条长命令

```bash
arshy run "cargo build --bin arshy"
```

长命令的返回在短命令字段之外，还会带 `error_count`、`warning_count`，以及解析出的 `events`：

```json
{
  "duration_ms": 130,
  "error_count": 0,
  "event_count": 0,
  "events": [],
  "exit_code": 0,
  "short_command": false,
  "status": "completed",
  "task_id": "06315a5b-658d-4dd8-8e34-731e61f6e9c7",
  "warning_count": 0
}
```

`short_command` 为 `false`，说明这次走的是结构化管线。构建干净时事件为空；有报错时事件会带上类型、严重级别和位置（见下一步）。

**长命令的等待策略**：auto 模式会同步等待最多 60 秒。超过 60 秒后降级为异步，返回 `status: "running"` 和 `task_id`——任务继续在 daemon 里执行，你可以用 `arshy list` / `arshy query` / `arshy tail` 跟进，或用 `arshy kill` 终止（见第 7 步）。

## 第 4 步：制造一个可解析的错误

用一条会报错的 Python 命令看看结构化事件。这里的管道让命令超过短路径的词数限制，强制走长路径：

```bash
arshy run 'python3 -c "import definitely_missing_module_xyz" | cat'
```

返回的事件里能看到解析器提取的结构：

```json
{
  "duration_ms": 219,
  "error_count": 2,
  "event_count": 2,
  "events": [
    {
      "location": { "file": "<string>", "line": 1 },
      "message": "Traceback (most recent call last):",
      "seq": 1,
      "severity": "error",
      "type": "diagnostic"
    },
    {
      "message": "No module named 'definitely_missing_module_xyz'",
      "seq": 3,
      "severity": "error",
      "type": "diagnostic"
    }
  ],
  "exit_code": 0,
  "primary_diagnostic": {
    "message": "No module named 'definitely_missing_module_xyz'",
    "seq": 3,
    "severity": "error",
    "type": "diagnostic"
  },
  "short_command": false,
  "status": "completed",
  "task_id": "e616c816-dc2f-4e25-b46d-17ed16f190ef",
  "warning_count": 0
}
```

注意：`primary_diagnostic` 给出一条代表性诊断证据，不对根因作额外推断；`location.file` 与 `location.line` 让 agent 能直接定位到出错位置。

## 第 5 步：只返回错误事件

在 agent 场景，通常只关心错误。加 `--errors-only`：

```bash
arshy run 'python3 -c "import definitely_missing_module_xyz" | cat' --errors-only
```

结果只保留 `severity` 为 `error` 的事件，过滤掉 warning/info 噪音。

## 第 6 步：切换输出格式

```bash
# 终端 UI（人类阅读，pretty 渲染统计框）
arshy run "echo hello" --format pretty

# 原始 JSON（agent 消费，显式指定）
arshy run "echo hello" --format json
```

`--format` 有三个取值：`auto`（默认）、`pretty`、`json`。`auto` 与 `json` 都输出 JSON；`pretty` 在终端渲染人可读的框式视图。

## 第 7 步：管理任务历史

每次运行都会生成一个 `task_id` 并写入 daemon 的存储（默认 `~/.local/share/arshy`）。

查看最近的任务：

```bash
arshy list --limit 5
```

按状态过滤：

```bash
arshy list --status running
```

查看某个任务的事件（用第 4 步返回的 task_id 替换）：

```bash
arshy query <task_id>
```

查看某个持久化任务的捕获文本：

```bash
arshy tail <task_id> --lines 50
```

### 终止一个超时任务

先用 `--timeout-ms` 演示超时（命令 3 秒后被 daemon 杀掉）：

```bash
arshy run "sleep 30" --timeout-ms 3000
```

预期返回 `status: "timeout"`、`exit_code: -1`，CLI 以退出码 255 结束。

再演示手动 kill。`&&` 会把命令推进长路径，先启动它：

```bash
arshy run "sleep 60 && echo done"
```

在另一个终端找到它的 task_id 并杀掉：

```bash
arshy list --status running
arshy kill <task_id>
```

被 kill 的任务返回 `status: "killed"`、`exit_code: -3`。注意：长命令的同步等待上限是 60 秒，所以要 kill 需在启动后的 60 秒内执行 `list` + `kill`。

## 第 8 步：看统计与影响分析

运行几条命令（包括上面的失败例子）后，查看聚合统计：

```bash
arshy stats                # 终端框式视图
arshy stats --format json  # 原始 JSON
```

pretty 视图会展示任务总数（按状态拆分）、事件数、失败率、平均时长、parser 覆盖率、去重节省的行数，以及命中次数最多的 parser（如 `cargo`、`cargo-test`、`python`）。JSON 里还能看到 `p50_duration_ms`、`p99_duration_ms`、`per_parser_usage` 等量化指标。

查看影响分析报告（需要有足够的历史数据，新安装请先跑几条命令再试）：

```bash
arshy analyze --format pretty
```

报告包含 Summary、quality-v2 分项指标、INFORMATION DENSITY 与 COMMAND PATTERNS 等区块。它们是显式 analytics 的工程指标，不是 token 节省估算。

### 标记任务用途

用 `--purpose` 给任务打标签，让统计区分测试负载与真实开发（未打标签的任务按真实开发计）：

```bash
arshy run "cargo test" --purpose dogfood
```

## 小结

你现在已经掌握：

- 短命令即时返回、长命令结构化解析的区别（`short_command` 字段）。
- 如何解读 `events`、`primary_diagnostic`、`error_count` 与 `warning_count`。
- `--format pretty/json`、`--errors-only`、`--timeout-ms` 的用法。
- 用 `list` / `query` / `tail` / `kill` 管理任务。
- 用 `stats` 与 `analyze` 观察执行效果。

下一步：把 arshy 接入你的 AI agent，让 agent 直接通过 `arshy_exec` /
`arshy_query` 执行命令——见[Agent 接入教程](setup-agent.md)。
