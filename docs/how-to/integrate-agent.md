# 把 arshy 接入 AI Agent

本文解决以下任务：查看 8 个一等 agent 的接入状态、用 `arshy setup` 接入（单个/全部）、用 `arshy init` 做项目层接入（`.arshy.toml`/`.mcp.json`/`AGENTS.md` 三层）、理解 bash 代理的原理与 opt-in 规则、给任意 MCP agent 手工接入（manual MCP）、用 `uninstall` 做到零残余。

集成实现见 `src/cli/integrate.rs`（8 个 agent 的注册表与各层编排）和 `src/cli/shell_wrapper.rs`（bash 代理、workspace init、Claude hook）。

## 1. 查看接入状态

```bash
arshy setup --status        # 等价: arshy integrate --status
```

输出表格（实测）：

```
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

8 个一等 agent 及接入机制（`all_agents()`，`src/cli/integrate.rs`）：

| agent id | 产品 | 机制 | 写入位置 |
|---|---|---|---|
| `claude-code` | Claude Code | PreToolUse hook + MCP | `~/.claude/settings.json`、`~/.claude.json`、工作区 `CLAUDE.md` |
| `cursor` | Cursor | MCP | `~/.cursor/mcp.json` |
| `vscode` | VS Code | MCP | `~/.vscode/mcp.json` |
| `antigravity` | Gemini CLI / Antigravity | MCP | `~/.gemini/config/mcp_config.json` |
| `codex` | OpenAI Codex | MCP + AGENTS.md | `~/.codex/config.toml`（`[mcp_servers.arshy]`）、工作区 `AGENTS.md` |
| `opencode` | OpenCode | AGENTS.md | 工作区 `AGENTS.md` |
| `aider` | Aider | AGENTS.md | 工作区 `AGENTS.md` |
| `workbuddy` | WorkBuddy | GUI PATH | 无配置文件改动，依赖 `~/.arshy/bin` 进入 GUI PATH |

`status` 列含义：`active`（已接入）/ `inactive`（已检测到 agent 但未接入）/ `not found`（本机未安装该 agent）。

## 2. 用 arshy setup 接入 agent

### 接入单个 agent

```bash
arshy setup codex        # 只接 Codex
```

### 接入所有已检测到的 agent

```bash
arshy setup              # 无参数 = 全部检测到的 agent
```

（`arshy integrate` 是 `setup` 的兼容别名，行为相同。）

执行内容（`integrate_all`，`src/cli/integrate.rs`）：

1. **L2 GUI PATH**：macOS 上 `launchctl setenv PATH`（立即）+ 写入 `~/Library/LaunchAgents/com.arshy.path.plist`（登录后重新生效）；Linux 写 `~/.config/environment.d/arshy.conf`；
2. **L0/L1 shim + 终端 PATH**：创建 `~/.arshy/bin/{sh,bash,zsh,claude-hook}` 符号链接指向 arshy，并在 `~/.zshrc`/`~/.bashrc` 追加 `export PATH="$HOME/.arshy/bin:$PATH"`；
3. **L3 各 agent 一等集成**：按上表写入对应配置文件/指令块。

无参数运行只对**检测到**的 agent 执行；`--agent <id>` 指定单个 agent（未检测到时打印 `⚠ not detected, skipped`）。

### 演练模式

```bash
arshy setup --dry-run        # 打印将要执行的步骤，不做修改
```

> 注意（当前版本的实测行为）：dry-run 仍会调用 Claude Code 的 `install_claude_hook`（该函数未受 dry-run 保护，`src/cli/shell_wrapper.rs`），因此会打印 `Registered Claude Code PreToolUse hook.` 并实际写入 `~/.claude/settings.json`；其余 L2/L3 变更会被跳过并打印 `(dry-run) no changes were made.`。

### 完成后

- 重启 agent / IDE（setup 输出 `Done. Restart your IDE / terminal to activate arshy.`）；
- 验证：`arshy setup --status` 中该 agent 变为 `active`，或 `arshy doctor --agent <id>`。

## 3. 用 arshy init 做项目层接入（三层）

`arshy init` 在**当前工作区**写入三层工件（`init_workspace_at`，`src/cli/shell_wrapper.rs`）：

```bash
cd /path/to/project
arshy init
```

1. **`.arshy.toml`** — bash 代理的 opt-in 标记（arshy 自有文件）：

   ```toml
   # Arshy workspace opt-in marker
   # Presence of this file (or an .arshy/ directory) in a workspace tells
   # arshy's shell hook to intercept commands here and start the daemon on demand.
   enabled = true
   ```

   也可以不建文件，直接放一个空的 `.arshy/` 目录，效果相同（`is_workspace_configured` 向上递归查找这两种标记之一）。

2. **`.mcp.json`** — 项目级 MCP 注册，任何支持项目 MCP 配置的 agent 都能发现：

   ```json
   {
     "mcpServers": {
       "arshy": {
         "type": "stdio",
         "command": "arshy",
         "args": ["--from-mcp"],
         "env": {}
       }
     }
   }
   ```

   `command` 保持 `"arshy"`（PATH 解析），因此文件可提交进仓库、跨机器可用；若本机 PATH 里没有 arshy，请改用绝对路径。

3. **`AGENTS.md`** — arshy 指令块，用 `<!-- arshy-agent-instructions -->` 标记包裹（幂等注入；`inject_agents_md`）。内容是"所有 shell 命令走 `arshy_exec` / `arshy run`"的操作指令，任何读取 AGENTS.md 的 agent（Codex、OpenCode、Aider…）都会遵循。

撤销（零残余）：

```bash
arshy init --undo
```

- 删除 `.arshy.toml`（arshy 自有文件）；
- 从 `.mcp.json` 移除 arshy 条目；若移除后只剩空容器则一并清理，文件为空则整个删除（保留用户其他 MCP server）；
- 从 `AGENTS.md` 删除两个 marker 之间的指令块；若文件只剩该块则整个删除。

## 4. bash 代理：原理与 opt-in

bash 代理是**兜底通道**：agent 不通过 MCP 而是直接 `bash -c "..."` 时，进程级的 `bash` 被 `~/.arshy/bin/bash` shim 拦截，改写为走 arshy 执行层；人类终端与其他未知进程不受影响。

### 判断是否拦截（`should_intercept`，`src/cli/shell_wrapper.rs`）

拦截条件同时满足：

1. 参数里有 `-c`（即 `bash -c "cmd"`）；
2. 不是 TTY 冲突场景（父进程为 vim/nvim/less/more/htop/tmux/screen 时放行）；
3. 不是递归调用（参数里含 `arshy`/`claude-hook` 时放行）；
4. **工作区已 opt-in**（`is_workspace_configured`：当前目录向上找到 `.arshy.toml` 或 `.arshy/`），**或**父进程是"明确桌面 agent"（codex、claude、cursor、vscode、code helper、gemini、antigravity、agy、opencode、aider、workbuddy 进程名）；
5. 调用方是 agent 环境：环境变量 `ARSHY_AGENT` / `CLAUDE_CODE` / `CLINE_AGENT` 已设置，或父进程命中第 4 条（明确 agent 或 agent/node/python 类进程）。

关键规则：

- **模糊父进程**（generic agent、node、python）**必须**有工作区标记才拦截——避免脚本/构建工具被意外接管；
- **其他 MCP agent**（Copilot CLI、Cline、Roo 等）不在被动拦截白名单里，走 `arshy init` 的项目标记或第 6 节 manual MCP 显式接入；
- **`ARSHY_BYPASS=1`** 环境变量可跳过拦截（Claude hook 改写命令时设置，防止递归）；
- 拦截失败会回退到真实 shell，并打印 `Arshy interception failed: ... Falling back to real shell...`。

### 启用/禁用 shim

```bash
arshy hook install        # 创建 ~/.arshy/bin shim + 终端 PATH + Claude hook + ~/.arshy/parsers
arshy hook uninstall      # 移除 shim 与 rc 行
```

`arshy setup` 无参数时也会准备 L0/L1（shim + PATH）。

## 5. Claude Code 专属：PreToolUse hook

`arshy setup claude-code`（或 `arshy install`）会在 `~/.claude/settings.json` 注册 Bash 的 PreToolUse hook（`src/cli/shell_wrapper.rs` 的 `install_claude_hook` / `run_claude_hook`）：

- 拦截 `Bash` 工具调用，把命令改写为 `ARSHY_BYPASS=1 arshy run '<escaped>'` 并以 `allow` 放行；
- 命令以 `arshy`/`arshyd`/`claude-hook` 开头时不改写（防递归）；
- 不可解析的输入默认 `allow`（不拦截）。

## 6. 任意 MCP agent：manual MCP 接入

任何支持 MCP 的 agent 都可以手工接入，方式二选一：

### 6.1 项目级（推荐，随仓库走）

```bash
cd /path/to/project
arshy init          # 生成 .mcp.json（含 arshy 条目）
```

或手工写 `.mcp.json`：

```json
{
  "mcpServers": {
    "arshy": {
      "type": "stdio",
      "command": "/absolute/path/to/arshy",
      "args": ["--from-mcp"],
      "env": {}
    }
  }
}
```

### 6.2 全局级（按 agent 的 MCP 配置格式）

注册为 stdio MCP server，命令为 `arshy`，参数 `--from-mcp`。示例（Claude Code 全局 `~/.claude.json`、Cursor `~/.cursor/mcp.json`、VS Code `.vscode/mcp.json`、Gemini `~/.gemini/config/mcp_config.json`、Codex `~/.codex/config.toml` 的 `[mcp_servers.arshy]`）：

```json
"mcpServers": {
  "arshy": {
    "type": "stdio",
    "command": "/Users/me/.local/bin/arshy",
    "args": ["--from-mcp"],
    "env": {}
  }
}
```

`arshy --from-mcp` 是 MCP stdio proxy：从 stdin 读 JSON-RPC，经 UDS 转发给 daemon，把结构化结果写回 stdout。注册后 agent 会看到两个工具：`arshy_exec`（run/cd/kill/list/tail 统一入口）与 `arshy_query`（事件查询）——工具定义见 `src/mcp/protocol.rs` 与 `src/mcp/instructions.rs`。

## 7. 诊断：arshy doctor

```bash
arshy doctor                  # 全量诊断
arshy doctor --agent codex    # 只看某个 agent
```

检查项（`src/cli/mod.rs` 的 `doctor`）：

1. `arshy`/`arshyd` 是否在 PATH；
2. daemon 是否运行、有无残留 spawn-lock（有且 daemon 未运行会自动清除）；
3. `~/.claude.json` / `~/.cursor/mcp.json` 是否有 arshy MCP 条目；
4. `~/.claude/settings.json` 是否包含 4 条权限：`mcp__arshy__arshy_exec`、`mcp__arshy__arshy_query`、`Bash(arshy *)`、`Bash(arshyd *)`；
5. macOS TCC 文件访问（Documents/Desktop/Downloads）；
6. shell hook 是否生效、当前工作区是否 opt-in、auto_start 是否开启（三者齐备时 agent `bash -c` 才会被拦截）；
7. 各 agent 集成状态（同 `setup --status`）。

## 8. 卸载：零残余

```bash
arshy uninstall                # 移除所有注册 + GUI PATH + shim + 已安装二进制
arshy uninstall --agent codex  # 只移除单个 agent（不碰全局 shim/GUI PATH）
```

`uninstall`（`src/cli/mod.rs` + `integrate::teardown_all` + `shell_wrapper::uninstall_hook`）：

- 从 `~/.claude.json`、`~/.cursor/mcp.json`、`~/.gemini/config/mcp_config.json` 移除 arshy 条目；
- 从 `~/.claude/settings.json` 移除 4 条权限；
- 撤销 GUI PATH（launchctl unload LaunchAgent / 删除 environment.d 配置）；
- 删除 `~/.arshy/bin` shim 与 shell rc 中的 hook 行；
- 若运行中的二进制位于 `~/.local/bin`，同时删除 `arshy`/`arshyd` 两个文件（开发构建 `target/...` 不动）；
- **保留**：任务/事件数据（`~/.local/share/arshy`，提示手工删除）与用户 parser（`~/.arshy/parsers`）。

`uninstall --agent <id>` 只调用该 agent 的 `uninstall`（如 Codex：移除 `~/.codex/config.toml` 中的 `[mcp_servers.arshy]` + 工作区 AGENTS.md 指令块）；未知 id 报错并列出合法 id 列表。
