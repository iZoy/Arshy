# 接入模型：两条路径，一个执行层

arshy 面对的现实是：每个 AI 编码代理（Claude Code、Cursor、Codex……）都有自己配置 shell 的方式，有的支持 MCP，有的只有 hook，有的什么都支持但配置格式各异。如果为每个代理做一套专用集成，就是 8 份互相漂移的代码。

`src/cli/integrate.rs` 的模块注释给出了答案，标题就叫"two paths, one command"：

1. **原生路径（MCP）**：把 arshy 注册为 MCP server，agent 直接调用 `arshy_exec`，拿到结构化结果，零运行时开销——这是首选；
2. **兜底路径（透明 bash 代理）**：进程级拦截 agent 发起的 `bash -c`，路由到同一个执行层——agent 什么都不用改。

两条路径通向同一个守护进程、同一个 `task/run`、同一套解析与安全。本文解释这两条路径如何选择、如何共存，以及"不做"了什么。

## 路径总览

```mermaid
flowchart TB
    subgraph Native[原生路径 · MCP]
        A1[Claude Code<br/>PreToolUse hook + MCP] --> MCP
        A2[Cursor / VS Code / Gemini CLI] -->|mcp.json| MCP
        A3[Codex] -->|config.toml + AGENTS.md| MCP
        MCP[arshy --from-mcp<br/>stdio 代理] --> Daemon[(arshyd)]
    end
    subgraph Proxy[兜底路径 · bash 代理]
        B1[任意 agent 的 bash -c] --> B2[~/.arshy/bin/bash 符号链接]
        B2 --> B3{拦截判定}
        B3 -->|命中| B4[task/run via UDS]
        B4 --> Daemon
        B3 -->|未命中| B5[/bin/bash 透传]
    end
```

## 原生路径：注册即用

MCP 注册的核心是一个 stdio server 条目：

```json
{
  "type": "stdio",
  "command": "<arshy 二进制路径>",
  "args": ["--from-mcp"],
  "env": {}
}
```

`integrate` 按代理把这条目写进各自认的配置文件：Claude Code 的 `~/.claude.json`、Cursor 的 `~/.cursor/mcp.json`、VS Code 的 `~/.vscode/mcp.json`、Gemini CLI 的 `~/.gemini/config/mcp_config.json`、Codex 的 `~/.codex/config.toml`（`[mcp_servers.arshy]` 块，追加式合并、保留用户注释）。Claude Code 还额外安装 PreToolUse hook（`settings.json` 里的 `claude-hook`），hook 把 bash 调用改写为 `ARSHY_BYPASS=1 arshy run ...`，见下文"防递归"。

对读 `AGENTS.md` 的代理（Codex、OpenCode、Aider），同时注入一段带边界标记的指令块，告诉 agent 用 `arshy_exec` 而不是 raw Bash，并给出 CLI fallback 与 `--errors-only` 等用法。

## 兜底路径：进程级 bash 代理

不是所有代理都支持 MCP，或不是所有命令都走 MCP 工具。bash 代理覆盖这部分：`~/.arshy/bin/` 下创建指向 arshy 二进制的符号链接（`sh`、`bash`、`zsh`、`claude-hook`），并把 `~/.arshy/bin` 前置到 PATH：

- 终端内：`~/.zshrc` / `~/.bashrc` 前置 PATH（`setup_terminal_path`）；
- GUI 会话：macOS 用 `launchctl setenv PATH` + 登录时重放的 LaunchAgent（`com.arshy.path.plist`），Linux 用 `~/.config/environment.d/arshy.conf`（systemd user session 拾取）——因为 Cursor.app、VS Code.app 这类桌面应用**从不 source shell rc**，这是唯一能让它们继承 PATH 的途径。

于是 agent 进程里任何 `bash -c "..."` 都会先命中 `~/.arshy/bin/bash` 符号链接，进入 `run_wrapper`。

### 拦截判定：能不猜就不猜

`run_wrapper` 的拦截条件按成本从低到高排列：

1. **`ARSHY_BYPASS=1`**：直接透传真实 shell——这是防递归的总开关（hook 改写后的命令自带该变量）；
2. **工作区 opt-in**：当前目录向上找 `.arshy.toml` 文件或 `.arshy/` 目录（`is_workspace_configured`），这是 bash 代理的显式授权；
3. **环境标记**：`ARSHY_AGENT` / `CLAUDE_CODE` / `CLINE_AGENT`——O(1) 的便宜检查；
4. **父进程名**：只有上面都没有时才 fork `ps` 查父进程名（`get_parent_process_name`），把昂贵的系统调用留给真正需要的路径。

最终决策 `should_intercept` 遵循三条规则：

- **明确的桌面 agent 直接拦截**：父进程名匹配白名单（codex、claude、cursor、vscode、code helper、gemini、antigravity、agy、opencode、aider、workbuddy）时，即使工作区没有 opt-in 也拦截——这是"代理的 bash 几乎全覆盖"的实现方式；
- **模糊父进程需要显式 opt-in**：名字含 `agent`/`node`/`python` 的父进程（脚本、构建工具、通用运行时）**必须**有工作区标记才拦截，避免用户的构建脚本被意外劫持；
- **人类环境放行**：父进程是 vim/nvim/less/more/htop/tmux/screen（直接控制 TTY 的交互程序）时绕过，避免 UI 冲突；参数里含 `arshy` 或 `claude-hook` 视为递归调用，放行。

判定失败只会"少拦截"，不会"错拦截"——透传真实 shell 是默认行为。

### 拦截后的行为

拦截后走 `intercept_and_run`：连接/拉起守护进程 → `task/run`（mode auto）→ 按结果形态输出。短命令打印原始输出（byte-for-byte，行为与原生 shell 一致）；长命令打印一行摘要 + 根因 + 变更文件 + 最多 8 条 error/warning 事件（`render_run_text`）；auto 模式降级为异步（超过 60s）时，代理最多耐心跟随 120s，仍在运行就打印 delegation hint 并以 0 退出——**守护进程继续执行**，agent 可用 `arshy query`/`arshy kill` 接管。代理进程即使被杀，任务也不中断，这是"透明代理"与"包装 bash"的本质区别。

## 8 个一等 agent

`all_agents()` 的注册表（顺序即 `integrate --all` 的优先级）：

| id | 名称 | 机制 |
|---|---|---|
| `claude-code` | Claude Code | PreToolUse hook + MCP + 工作区 CLAUDE.md |
| `cursor` | Cursor | MCP（`~/.cursor/mcp.json`） |
| `vscode` | VS Code | MCP（`~/.vscode/mcp.json`） |
| `antigravity` | Gemini CLI / Antigravity | MCP（`~/.gemini/config/mcp_config.json`） |
| `codex` | OpenAI Codex | MCP（`~/.codex/config.toml`）+ AGENTS.md |
| `opencode` | OpenCode | AGENTS.md |
| `aider` | Aider | AGENTS.md |
| `workbuddy` | WorkBuddy | GUI PATH 层（不写任何配置文件） |

每个实现都有 `detect`（装没装）/ `integrate` / `uninstall` / `status` 四个方法，`arshy doctor --agent <id>` 就是逐代理跑 `status` 并给修复建议。**白名单外**的 MCP 代理（Copilot CLI、Cline、Roo 等）不被动拦截——它们通过项目级 `.mcp.json`（任何支持项目 MCP 的代理都能读到）或工作区标记显式接入。

## 三层协议路径：agent 无关的接入

`arshy init` 在一个工作区落下三个文件，每一层回答一个不同的问题（`src/cli/shell_wrapper.rs`）：

| 层 | 文件 | 回答的问题 | 谁消费 |
|---|---|---|---|
| 1 | `.arshy.toml` | 这个工作区允许 bash 代理拦截吗？ | 代理自身的 `is_workspace_configured` |
| 2 | `.mcp.json` | 任何 MCP 代理如何发现 arshy？ | 任意读项目 MCP 配置的 agent（`command: "arshy"` 保持 PATH 可移植） |
| 3 | `AGENTS.md` | agent 如何使用 arshy？ | 读 AGENTS.md 的 agent（Codex、OpenCode 等） |

三层各自独立、各自幂等：`.arshy.toml` 是 arshy 独有的 opt-in 标记（内容即注释说明）；`.mcp.json` 用 `merge_mcp_json` 幂等合并；`AGENTS.md` 指令块用 `<!-- arshy-agent-instructions -->` 成对标记包裹。`arshy init --undo` 反向移除三层。

为什么是三层而不是一个"超级配置文件"？因为三者的消费方不同：bash 代理是进程级行为（需要目录标记），MCP 是协议级发现（需要标准位置），指令是提示词级引导（需要 agent 的指令文件）。把三层塞进一个文件会让某一类消费方读不到它需要的那一层。

## 零残余卸载哲学

集成必须可逆到"像从未发生过"——这是代码里的硬约束（"Idempotent, reversible (every integrate has a matching uninstall), and a --dry-run mode"）。零残余落实在几个具体机制上：

- **成对标记**：AGENTS.md 指令块从第一个 marker 到第二个 marker 整段切除；`.arshy.toml` 是 arshy 自有的，直接删除；
- **空容器清理**：`remove_mcp_json` 移除条目后，如果 `mcpServers` 空了就删掉该键，如果整个文件只剩 arshy 留下的内容就删掉文件——用户自己的内容永远保留；
- **Codex 配置按块删除**：从 `[mcp_servers.arshy]` 表头到下一个表头，连同 arshy 管理的注释行，其余内容逐字保留；若文件只剩 arshy 块则删文件；
- **PATH 还原**：`teardown_gui_path` 卸载 LaunchAgent / 删除 env.d 并还原先前的 launchd PATH；
- **幂等**：重复 integrate 不产生重复条目（检测已存在则跳过），重复 uninstall 不报错。

## 为什么不做 Codex PreToolUse deny 重定向

一个"顺理成章"的集成方式是把 Codex 的 PreToolUse 规则配成"任何 Bash 调用重定向到 arshy"。模块注释解释了为什么不：

> Codex's PreToolUse only supports deny/block (no `updatedInput` rewrite), so a redirect would interrupt the agent instead of being seamless.

PreToolUse 在 Codex 里只能**拒绝**（deny/block），不能改写请求参数（没有 `updatedInput`）。用 deny 重定向意味着：agent 每次想跑 bash，先被拒绝一次，再被要求改用别的工具——体验是打断而不是无缝。Codex 因此走"原生路径（MCP + AGENTS.md 指令）+ 兜底路径（bash 代理）"，两者都不需要 PreToolUse deny。

Claude Code 的 hook 是唯一使用改写机制的：它的 hook 系统支持在 PreToolUse 中重写命令为 `ARSHY_BYPASS=1 arshy run ...`，这是平台能力差异下的"能用改写就用改写"。

## 权衡总结

| 权衡 | 选择 | 原因 |
|---|---|---|
| 每代理专用集成 vs 两条通用路径 | 两条通用路径 | 一套机制覆盖所有代理，避免 8 份漂移代码 |
| 主动拦截 vs 被动等待 | 白名单主动 + 模糊场景 opt-in | 明确 agent 零配置接管；脚本/构建工具不被意外劫持 |
| 环境变量检测 vs 父进程检测 | 先用 env，必要时 ps | O(1) 优先，ps 成本只在真正需要时付出 |
| 项目级 vs 用户级配置 | 三层都有 | 项目文件可提交进仓库、团队共享；用户级文件覆盖本机 |
| 卸载 | 零残余 | 集成是"借用"用户的配置，必须还回去 |
| PreToolUse deny 重定向 | 不做 | 平台不支持无缝改写，只会打断 agent |
