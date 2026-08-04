# arshy 多 Agent 自动接入方案（omni-integration）

**版本：** 1.0
**日期：** 2026-07-29
**状态：** 已实施（2026-07-29）。用户向说明见 [`omni-integration.md`](../../explanation/integration-model.md)。

---

## 0. 目标与不可逾越的红线

**目标：** 用户执行一次安装/集成命令后，其机器上"每一个 agent 环境"在运行 shell 命令时，透明地走 arshy（而非裸 bash），且 arshy 守护进程按既有的 on-demand 模型自动拉起。

**原则：**

1. **不替换任何系统二进制**（红线）。绝不 `mv`/`ln` 覆盖 `/bin/bash`、`/bin/zsh`。所有改动仅限于 `~/.arshy/`、`~/.config/environment.d/`、`~/Library/LaunchAgents/`、各 agent 的 dotfiles。
2. **每 agent 选最优机制，PATH shim 作通用兜底。** 有 API 的 agent 走 API，无 API 的靠 PATH 透明拦截。
3. **幂等、可卸载、可诊断。** 同命令重复执行无副作用；`arshy uninstall` 精确反向还原；`arshy doctor`/`integrate --status` 能逐 agent 报告状态。

---

## 1. 总体架构：4 层 + 诊断

| 层 | 名称 | 作用 | 当前状态 |
|----|------|------|---------|
| L0 | shim 软链 | `~/.arshy/bin/{bash,zsh,sh}` → arshy，靠 PATH 解析顺序抢先命中 | ✅ 已有 |
| L1 | 终端 PATH | `.zshrc`/`.bashrc` prepend `~/.arshy/bin` | ✅ 已有 |
| L2 | GUI 会话 PATH 持久 | 让 Dock/Finder 拉起的 GUI 应用也继承含 `~/.arshy/bin` 的 PATH | ❌ 缺失 |
| L3 | 每 agent 一等集成 | MCP / AGENTS.md / PreToolUse，按 agent 类型自动 merge | △ 部分（仅 Claude/Cursor/Codex 文档） |
| L4 | doctor 逐 agent 诊断 | 报告每个 agent 是否真正走 arshy、缺哪一层 | △ 部分（仅 L0/L1 部分） |

**关键认知：** L0 只有在"进程 exec `bash`/`sh`/`zsh` 且环境 PATH 含 `~/.arshy/bin`"时才生效。L2 解决 GUI 应用（WorkBuddy、Cursor.app、VS Code.app）不 source rc 导致 PATH 缺失的问题——这是当前 dogfooding 缺口的根因。

---

## 2. 各层详细设计

### L0 — shim 软链（复用并修正）

- 已有 `install_hook()` 在 `~/.arshy/bin/` 建 `sh`/`bash`/`zsh`/`claude-hook` 软链指向 `current_exe()`。
- **修正点：** `current_exe()` 在开发期是 `target/debug/arshy`（未签名、易失效）。改为指向**稳定安装路径**（install.sh 已拷贝到 `~/.local/bin/arshy`），即 shim 统一 `→ $HOME/.local/bin/arshy`。`hook install` 接受可选 `--arshy-bin` 参数，默认探测 `~/.local/bin/arshy` / `/usr/local/bin/arshy`。
- 软链解析目标用 `readlink` 校验指向 arshy（已有 `is_hook_active` 逻辑）。

### L1 — 终端 PATH（复用并扩展）

- 已有 `.zshrc`/`.bashrc` prepend。
- **扩展：** 增加 `.zprofile` / `.profile`（login shell 覆盖），去重逻辑复用。

### L2 — GUI 会话 PATH 持久（新增，核心）

**macOS：**
- 立即生效：`launchctl setenv PATH "$HOME/.arshy/bin:$(launchctl getenv PATH 2>/dev/null)"`。对 setenv 之后启动的 GUI 进程生效。
- 跨重启持久：写 `~/Library/LaunchAgents/com.arshy.path.plist`，`ProgramArguments` 跑同一 setenv 脚本，在用户登录时执行（`RunAtLogin`，非 `KeepAlive`，避免常驻）。
- 卸载：`launchctl unsetenv PATH` + 删 plist + `launchctl unload`。

**Linux：**
- systemd 用户会话：写 `~/.config/environment.d/arshy.conf`：`PATH=$HOME/.arshy/bin:$PATH`（或读取现有 PATH 追加）。GNOME/KDE 登录时读取。
- 非 systemd 桌面：尽力写 `~/.profile` / `~/.xprofile`。
- 卸载：删 `arshy.conf`。

**风险与缓解：**
- `launchctl setenv PATH` 是**用户级全局** PATH 修改。我们用"仅 prepend 一个已存在的目录"，不破坏原有 PATH 顺序，副作用低；卸载精确还原。
- 提供 `--dry-run` 预览将要写入的 PATH 与 plist 内容，供审计。

### L3 — 每 agent 一等集成（新增注册表）

设计 `Agent` trait：`detect() -> bool`、`integrate() -> Result`、`uninstall() -> Result`、`mechanism() -> &str`、`status() -> AgentStatus`。

注册表（按优先级）：

| Agent | 检测依据 | 最优机制 | 自动动作 |
|-------|---------|---------|---------|
| Claude Code | `~/.claude/settings.json` 或 `claude` 二进制 | PreToolUse hook + CLAUDE.md | 已有 `install_claude_hook`；追加 CLAUDE.md 段 |
| Cursor | `~/.cursor` 或 Cursor.app | MCP server | merge `~/.cursor/mcp.json`：`arshy --from-mcp` |
| VS Code / Cline / Roo | `code` 二进制 / `~/.vscode*` | MCP server | merge `.vscode/mcp.json` 或全局 `settings.json` 的 `mcp.servers` |
| OpenCode | `opencode` 二进制 | AGENTS.md + MCP（若支持） | 注入 `AGENTS.md`；有 MCP 配置则 merge |
| Codex | `codex` 二进制 / `~/.codex` | AGENTS.md 指令 | 注入项目 `AGENTS.md`（已有文档方案） |
| Gemini CLI | `gemini` 二进制 | `GEMINI.md` + MCP（若支持） | 注入 `GEMINI.md` |
| Aider | `aider` 二进制 | `.aider.conf.yml` / AGENTS.md | 注入说明段 |
| WorkBuddy（本应用） | 运行进程 / 配置 | 依赖 L2 + 其 Bash 工具配置 | 见 §4 诚实边界 |
| 通用兜底 | — | L0+L1+L2 | 仅靠 PATH 透明拦截 |

每个 `integrate()` 写文件前**备份**、**幂等**（已含 arshy 段则跳过）、`uninstall()` 精确反向。

### L4 — doctor 逐 agent 诊断（扩展）

对注册表每个 agent 检测并输出表格：

```
Agent        | Mechanism      | Active | Missing Layer | Fix
-------------|----------------|--------|---------------|----
Claude Code  | PreToolUse     | ✓      | —             |
Cursor       | MCP            | ✗      | L3            | arshy integrate --agent cursor
VS Code      | MCP            | —      | not detected  |
WorkBuddy    | GUI PATH       | ✗      | L2            | arshy integrate (re-run)
Generic bash | PATH shim      | ✓      | —             |
```

复用现有 `is_hook_active()`、workspace marker 检测、auto_start 检测，扩展为遍历注册表。

---

## 3. 命令入口

- `arshy integrate [--agent <name>|--all]`：应用 L2 + 对每个 detect 到的 agent 应用 L3；幂等。
- `arshy integrate --status`：逐 agent 视图（等同 L4 doctor 子集）。
- `arshy uninstall`：反向卸载 L0–L3，还原 PATH（launchctl unsetenv / 删 env.d / 删 LaunchAgent / 还原 rc / 删 shim）。
- `arshy hook install`：保留为 L0/L1 子集（向后兼容），内部改为调用统一的 PATH 设置函数。

---

## 4. 失效模式与诚实边界（审计重点）

1. **绝对路径硬编码 + 无 API 的 agent：** 若 agent 用 `Command::new("/bin/bash")`，PATH 不参与解析，shim 永不被命中，且无 hook/MCP/AGENTS 可注入。此类**无解**，只能提示用户在其设置里把"shell 可执行文件"指向 `~/.arshy/bin/bash`。README/doctor 需明确列出。
2. **WorkBuddy 本应用：** 取决于其 Bash 工具实现——(a) 是否继承 launchd 的 PATH（L2 解决）；(b) 是否用名字解析 shell 而非绝对路径。L2 解决 (a)；(b) 需 WorkBuddy 自身配合，或改用 WorkBuddy 的 MCP 接入 arshy。诚实标注：本方案不能 100% 保证 WorkBuddy 的 Bash 工具走 arshy，除非其使用 PATH 解析。
3. **launchctl 全局 PATH 副作用：** 见 §2 L2 风险缓解。
4. **MCP vs shim 的分工：** 走 MCP 的 agent（Cursor/Codex 等）其"bash 替换"由 MCP server 配置完成，shim 对其透明拦截路径无意义；shim 仅作为"裸 shell-out"agent 的兜底。方案不冲突，但需在文档中澄清，避免用户误以为 shim 是唯一切入点。

---

## 5. 实现步骤（任务分解）

1. 新增 `src/cli/integrate.rs`：`Agent` trait + 各 agent 实现（detect/integrate/uninstall/status）。
2. 新增 GUI PATH 层：`setup_gui_path()` / `teardown_gui_path()`（macOS launchctl+plist，Linux env.d）。
3. 修正 `hook install`：shim 指向稳定安装路径；内部复用统一 PATH 设置（含 L2）。
4. 扩展 `doctor`：遍历注册表输出逐 agent 表格（L4）。
5. CLI 入口：`arshy integrate` / `arshy integrate --status` / `arshy uninstall` 增强。
6. 测试：每个 agent 的 integrate/uninstall 单测（tempdir 模拟 dotfiles）+ 幂等性；GUI PATH 层用 `--dry-run` 特征探测（CI 不真跑 launchctl）。
7. 文档：`docs/explanation/integration-model.md`（用户向）、本设计稿定稿。

---

## 6. 验证计划

- **单元：** 注册表 merge/幂等/卸载；shim 指向校验。
- **手动 e2e（macOS）：** `arshy integrate --all` → 开新 Terminal + 新 Cursor.app + 新 WorkBuddy 会话，分别跑 `bash -c 'echo $PATH'`，验证 `~/.arshy/bin` 在前；`arshy integrate --status` 全绿。
- **卸载：** `arshy uninstall` 后 `launchctl getenv PATH` 不含 `~/.arshy/bin`、rc 还原、无残留 shim。

---

## 7. 风险与回滚

- 每步写前备份；卸载精确反向。
- LaunchAgent 持久 PATH 若写错可能导致登录 PATH 异常——用最小 prepend + `--dry-run` 预览。
- 所有改动限于用户 home 下的 arshy/dotfile 目录，不触及系统目录。
