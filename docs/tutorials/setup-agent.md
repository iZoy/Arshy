# 把 Codex 接入 arshy

本教程带你把一个 AI agent（以 OpenAI Codex 为例）接入 arshy，并端到端验证成功：agent 会话里出现 `arshy_exec` / `arshy_query` 两个 MCP 工具，`arshy doctor --agent codex` 全绿，最后你还能无残留地卸载。

前置条件：已完成[安装 arshy](install.md)，`arshy --version` 正常；已安装 Codex（`~/.codex` 目录存在，或 `codex` 在 PATH 中）。

## 背景：接入的两条路径

arshy 对每个已知 agent 采用两条路径（事实来自 `src/cli/integrate.rs`）：

1. **原生路径（MCP）**：把 arshy 注册为 agent 的 MCP server，agent 直接调用 `arshy_exec` / `arshy_query`，拿到结构化结果，零额外开销。
2. **兜底路径（透明 bash 代理）**：`~/.arshy/bin` 下的 `sh`/`bash`/`zsh` 软链拦截 agent 发起的 `bash -c`，路由到同一个执行层。人类和未知进程不受影响。

arshy 支持 8 个 agent，机制各不相同：

| agent id | 机制 |
| --- | --- |
| `codex` | MCP + AGENTS.md |
| `claude-code` | PreToolUse + MCP |
| `cursor` | MCP |
| `vscode` | MCP |
| `antigravity` | MCP |
| `opencode` | AGENTS.md |
| `aider` | AGENTS.md |
| `workbuddy` | GUI PATH |

统一入口是 `arshy setup`（`arshy integrate` 是它的兼容别名）。接入是幂等且可逆的：每个 `setup` 都有对应的 `uninstall`，且提供 `--dry-run` 预演。

## 第 1 步：查看各 agent 的检测状态

```bash
arshy setup --status
```

预期输出是一张表格：

```text
arshy agent integrations

AGENT          MECHANISM              STATUS         NOTE
──────────────────────────────────────────────────────────────────────
claude-code    PreToolUse + MCP       active         arshy reachable via Claude Code
cursor         MCP                    inactive       run: arshy integrate --agent cursor
vscode         MCP                    inactive       run: arshy integrate --agent vscode
antigravity    MCP                    inactive       run: arshy integrate --agent antigravity
codex          MCP + AGENTS.md        inactive       run: arshy integrate --agent codex
opencode       AGENTS.md              inactive       add arshy instructions to your project AGENTS.md
aider          AGENTS.md              not found      add arshy instructions to your project AGENTS.md
workbuddy      GUI PATH               inactive       run: arshy integrate (then restart WorkBuddy)
```

- `not found`：机器上没有检测到该 agent（Codex 的检测条件是 `~/.codex` 存在或 `codex` 在 PATH）。
- `inactive`：已检测到但还没接入。
- `active`：已接入。

## 第 2 步：预演（不修改任何文件）

```bash
arshy setup codex --dry-run
```

预期输出列出将要执行的动作，并以 `(dry-run) no changes were made.` 结尾：

```text
  ✓ [OpenAI Codex] Codex MCP server → /Users/<you>/.codex/config.toml
  ✓ [OpenAI Codex] injected arshy instructions into <cwd>/AGENTS.md

(dry-run) no changes were made.
```

对 Codex 来说，`setup` 只做两件事：

1. 在 `~/.codex/config.toml` 追加（append-only，保留你的注释和其他 `[mcp_servers.*]`）：

   ```toml
   # arshy: structured execution layer (managed by `arshy integrate`)
   [mcp_servers.arshy]
   command = "/path/to/arshy"
   args = ["--from-mcp"]
   ```

2. 在当前目录的 `AGENTS.md` 里注入一段以 `<!-- arshy-agent-instructions -->` 标记包裹的指令块，告诉 agent 用 `arshy_exec` 执行命令。

## 第 3 步：执行接入

```bash
arshy setup codex
```

预期输出（去掉 `(dry-run)` 提示）：

```text
  ✓ [OpenAI Codex] Codex MCP server → /Users/<you>/.codex/config.toml
  ✓ [OpenAI Codex] injected arshy instructions into /Users/<you>/Documents/your-project/AGENTS.md

Done. Restart your IDE / terminal to activate arshy.
```

验证写入结果：

```bash
cat ~/.codex/config.toml
```

文件末尾应出现 `[mcp_servers.arshy]` 块，且 `command` 指向你机器上 arshy 的绝对路径。

## 第 4 步：重启 Codex

**必须重启**。Codex 在启动时加载 `~/.codex/config.toml` 里的 MCP server；不重启就不会看到新工具。

重启后打开一个新的 Codex 会话，确认 agent 能看到两个工具：`arshy_exec` 与 `arshy_query`。

## 第 5 步：验证接入

```bash
arshy doctor --agent codex
```

doctor 只检查 Codex（其余 agent 会被跳过）。它逐项报告：

- **1. Binaries**：`arshy` / `arshyd` 在 PATH 中。
- **2. Daemon**：daemon 在运行（`arshy run` 会按需自动启动，一般无需手动处理）。
- **3. MCP server config**：`~/.claude.json` / `~/.cursor/mcp.json` 中的 arshy 注册（Codex 的注册在 `~/.codex/config.toml`，由 5.6 节报告）。
- **4. Permissions**：`mcp__arshy__arshy_exec`、`mcp__arshy__arshy_query`、`Bash(arshy *)`、`Bash(arshyd *)` 等权限。
- **5. Filesystem access**：macOS TCC 目录访问限制。
- **5.5 Agent shell interception**：shell 钩子、当前工作区是否 opt-in（`.arshy.toml` / `.arshy/`）、daemon auto-start 是否开启。
- **5.6 Agent integrations**：本机各 agent 的接入状态。

**完成标准**：最后一行 `failed` 为 0：

```text
  X passed, 0 warnings, 0 failed

  Everything looks good! Restart your IDE to activate arshy.
```

如果 `codex` 这一行仍显示失败（`run: arshy integrate --agent codex`），回到第 3 步重新执行 `arshy setup codex`，并确认你是在包含 `AGENTS.md` 的项目目录里运行的。

## 第 6 步（可选）：初始化项目工作区

`arshy init` 在**当前目录**创建三样东西，任何支持项目级 MCP 的 agent 都能识别，不限于 Codex：

```bash
arshy init
```

预期输出：

```text
Initialized arshy workspace: <cwd>
  ✓ <cwd>/.arshy.toml
  MCP server `arshy` → <cwd>/.mcp.json
  injected arshy instructions into <cwd>/AGENTS.md
Any MCP-capable agent in this project now sees arshy_exec/arshy_query;          the bash proxy is active for agent `bash -c` calls.
Undo with: `arshy init --undo`
```

创建的内容：

1. `.arshy.toml`——bash 代理 opt-in 标记（`.arshy/` 目录也能触发，二选一）。
2. `.mcp.json`——项目级 MCP 注册，`command` 用 PATH 解析的 `arshy`，`args` 为 `["--from-mcp"]`，可安全提交到仓库。
3. `AGENTS.md`——arshy 指令块（幂等，重复执行不重复注入）。

撤销项目层接入（零残留：只删除 arshy 自己的标记，保留你的其他内容）：

```bash
arshy init --undo
```

### 补上 shell 钩子（可选但推荐）

想让 agent 的 `bash -c` 也被 arshy 拦截（兜底路径），运行：

```bash
arshy hook install
```

它会在 `~/.arshy/bin` 创建指向 arshy 的 `sh` / `bash` / `zsh` / `claude-hook` 软链，并在 `~/.zshrc` / `~/.bashrc` 前置 `~/.arshy/bin`，同时创建自定义 parser 目录 `~/.arshy/parsers/`。撤销用 `arshy hook uninstall`。

如果想一次接入**所有已检测到的 agent**（含 GUI PATH 持久化，桌面启动的 agent 也能继承），直接运行不带参数的 `arshy setup`：

```bash
arshy setup
```

## 第 7 步：在 Codex 里使用 arshy

接入后，agent 会话里的命令应通过 `arshy_exec` 执行：

- 运行命令：`arshy_exec(action: "run", command: "cargo test")`
- 切换会话工作目录：`arshy_exec(action: "cd", command: "/absolute/path")`——之后的 run 都继承该目录；`cwd` 参数可做一次性覆盖
- 其他动作：`kill`（停止任务）、`list`（最近任务）、`tail`（查看输出）、`subscribe`（等待任务完成）
- 模式：`mode: "auto"`（默认，智能区分短/长）、`sync`（等待完成）、`async`（立即返回 task_id）
- 查询事件：`arshy_query`——传 `task_id` 查单个任务；省略 `task_id` 则跨任务搜索（结果带 `task_id`），可按 `event_type` / `severity` / `code` / `file` 过滤
- 兜底：`arshy_exec` 返回 `DaemonUnreachable` 时，agent 可以用 Bash 做一次性回退

## 第 8 步：卸载

### 只卸载 Codex（零残留）

```bash
arshy uninstall --agent codex
```

只动 Codex 自己的配置：从 `~/.codex/config.toml` 移除 `[mcp_servers.arshy]` 块（若文件只剩 arshy 内容则整个删除），并从当前目录的 `AGENTS.md` 移除 arshy 指令块。全局钩子、其他 agent 不受影响。

### 完全卸载

```bash
arshy uninstall
```

依次完成：移除各 MCP 注册与权限、还原 GUI PATH 层（macOS LaunchAgent / Linux environment.d）、移除 `~/.arshy/bin` 软链与 shell rc 里的钩子行、删除安装在 `~/.local/bin` 的 arshy/arshyd 二进制。保留两样东西：自定义 parser（`~/.arshy/parsers/`）与任务数据（`~/.local/share/arshy`，需手动删除才是彻底清除）。

## 故障排查

| 症状 | 处理 |
| --- | --- |
| `setup --status` 里 codex 显示 `not found` | 确认 `~/.codex` 存在或 `codex` 在 PATH；否则 setup 会跳过（提示 `not detected, skipped`） |
| 重启后 agent 仍看不到 `arshy_exec` | 运行 `arshy doctor --agent codex`，按 5.6 节提示重新 `arshy setup codex`；确认 `~/.codex/config.toml` 里 `command` 指向真实存在的 arshy 绝对路径 |
| `arshy setup codex --agent codex` 报错 | `setup` 的 agent 是位置参数：`arshy setup codex`；`--agent` 只用于别名 `arshy integrate --agent codex` 和 `arshy doctor --agent codex` |
| `unknown agent` 报错 | agent id 写错；合法 id 见第 1 步表格 |
| doctor 显示 daemon 未运行 | `arshy daemon start`，或直接 `arshy run "echo hi"` 触发自动启动 |
| doctor 显示工作区未 opt-in | 在项目目录运行 `arshy init` |
| 想先看效果再落地 | 所有接入命令都支持 `--dry-run`（`arshy setup codex --dry-run`、`arshy setup --dry-run`） |

## 小结

你现在已经完成：查看检测状态 → 预演 → 接入 Codex（`~/.codex/config.toml` + 项目 `AGENTS.md`）→ 重启 → `arshy doctor --agent codex` 验证 → 在 agent 中使用 `arshy_exec` / `arshy_query` →（可选）卸载。接入是幂等、可逆、可预演的，随时可以用 `arshy setup --status` 查看当前状态。
