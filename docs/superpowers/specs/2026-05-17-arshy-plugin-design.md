# Arshy Claude Code Plugin (方案 B)

## 概述

将 arshy 打包为标准的 Claude Code 插件，用户通过插件市场一键安装即可使用结构化 Shell 执行能力。预编译二进制分发 + daemon 按需启停，零配置开箱即用。

## 插件结构

```
.claude/plugins/cache/claude-plugins-official/arshy/
├── .claude-plugin/
│   └── plugin.json          # 元数据
├── .mcp.json                 # MCP 服务器配置
├── skills/
│   └── arshy-setup/
│       └── SKILL.md          # 安装诊断技能（首次加载自动触发）
└── README.md
```

### plugin.json

```json
{
  "name": "arshy",
  "version": "0.1.0",
  "description": "Structured shell execution — smart sync, audit log, command filtering, and task lifecycle management. Replaces raw Bash with typed, queryable command execution.",
  "author": {
    "name": "izoy"
  }
}
```

### .mcp.json

```json
{
  "mcpServers": {
    "arshy": {
      "type": "stdio",
      "command": "${HOME}/.cargo/bin/arshy",
      "args": ["--from-mcp"],
      "env": {}
    }
  }
}
```

MCP 工具模型保持完整的 2-tool 模型：`arshy_exec`（统一 run/cd/kill/list/tail/subscribe）+ `arshy_query`（结构化事件查询）。工具 schema 和 instructions 从 `src/mcp/instructions.rs` 生成，随插件分发一份静态副本供 MCP 初始化使用。

### arshy-setup 技能

用户输入 `/arshy-setup` 或 MCP 连接失败（`DaemonUnreachable`）时触发。

1. **检测二进制**：检查 `~/.cargo/bin/arshyd` 和 `~/.cargo/bin/arshy` 是否存在
2. **缺失时下载**：从 GitHub Releases（`izoy/arshy`）拉取对应平台的预编译 tar.gz
3. **校验完整性**：SHA256 checksum 验证
4. **解压安装**：安装到 `~/.cargo/bin/`，设 `chmod +x`
5. **验证可用**：`arshy status` 确认 daemon 可启动

技能描述字段应包含 "setup arshy"、"install arshy"、"configure arshy"、"arshy not found" 等触发短语。

## Daemon 空闲退出

### 现状

当前 proxy（`arshy --from-mcp`）通过 `connect_or_start()` 连接 daemon，daemon 启动后常驻内存，永不主动退出。用户关闭 Claude Code 时 proxy 退出，但 daemon 可能被遗留在后台。

### 目标

- MCP 活跃期间 daemon 保持运行（响应时延 < 10ms）
- 连续 N 分钟无 MCP 工具调用后，daemon 自动优雅退出
- 下次 MCP 调用到达时，proxy 自动通过 `connect_or_start()` 唤醒 daemon
- 唤醒过程对 Agent 透明（首次调用可能增加 ~50ms 启动延迟）

### 实现

在 `src/proxy/mod.rs` 的 MCP 主循环中加入空闲跟踪：

```
主循环入口：每次处理完一个 MCP 请求后，更新 last_activity = Instant::now()

在等待 stdin 的 select! 分支出中增加超时信号：
  stdin 可读  → 处理请求
  空闲超时   → 发送 daemon/shutdown → 标记 daemon_alive = false
```

改动范围：
- `src/proxy/mod.rs`：主循环 idle 分支（约 30 行）
- 默认空闲超时：300 秒（5 分钟），通过环境变量 `ARSHY_IDLE_TIMEOUT_SECS` 覆盖
- 发送 `daemon/shutdown` 后不立即退出 proxy —— proxy 继续运行等待下一次 MCP 请求
- 下次请求到达时，`connect_or_start()` 检测到 daemon 未连接，自动重新启动

### 边界情况

| 场景 | 行为 |
|------|------|
| 空闲期间有运行中的任务 | 跳过 shutdown，等任务完成后重试 |
| daemon 已在退出中 | `daemon/shutdown` 幂等，重复发送无副作用 |
| proxy 收到 SIGTERM | 正常退出前发送 `daemon/shutdown` 清理 |
| 首次安装无 daemon | `connect_or_start()` 直接启动，与空闲退出无关 |

## 二进制分发

### 构建

CI（GitHub Actions）在每次版本 tag 推送时：

1. `cargo build --release` on macOS (arm64) 和 Linux (x86_64)
2. 生成 SHA256 checksum
3. 打包 `tar.gz`：`arshy`, `arshyd` 两个二进制
4. 发布到 GitHub Releases，命名规则：
   - `arshy-v0.1.0-x86_64-apple-darwin.tar.gz`
   - `arshy-v0.1.0-x86_64-unknown-linux-gnu.tar.gz`

### 下载

安装技能通过 `uname -s` / `uname -m` 检测平台，拼接下载 URL：

```
https://github.com/izoy/arshy/releases/download/v{VERSION}/arshy-v{VERSION}-{target}.tar.gz
```

解压到 `~/.cargo/bin/`。如需多用户共享，可配置 `ARSHY_INSTALL_DIR` 覆盖。

### 更新

每次插件加载时，arshy-setup 技能对比本地版本（`arshy --version`）和 GitHub 最新 release。如有新版本，询问用户是否更新。更新时覆盖二进制，但保留用户配置和任务历史。

## 错误处理

| 错误场景 | 用户可见表现 | 恢复路径 |
|----------|-------------|---------|
| 二进制下载失败 | `/arshy-setup` 报告网络错误 + 重试建议 | 手动下载链接 + `ARSHY_INSTALL_DIR` 指引 |
| checksum 不匹配 | 安装中止，警告用户 | 清除缓存文件，重新下载 |
| daemon 无法启动 | MCP 连接失败，Agent 回退到 Bash | `arshy-setup` 诊断：检查端口、权限、日志 |
| 空闲退出后唤醒超时 | 首次 MCP 调用返回 `DaemonUnreachable` | Agent 重试一次，proxy 自动重连 |
| 二进制无执行权限 | daemon 启动失败 | arshy-setup 自动执行 `chmod +x` |
| 正在运行任务时空闲超时 | 跳过 shutdown，等待任务完成 | 下一次空闲周期再检查 |

## 测试策略

### 单元/集成测试（已有，255 个，不变）

所有现有测试保持通过。空闲退出逻辑新增 2 个测试：

1. `e2e_idle_exit_daemon`：模拟 MCP 请求 → 等待超时 → 验证 daemon 退出 → 新请求到达 → daemon 自动唤醒
2. `e2e_idle_exit_skipped_when_task_running`：有运行中的任务 → 空闲超时不杀 daemon

### 手动验收

```bash
# 1. 启动 proxy
arshy --from-mcp --idle-timeout 10 &

# 2. 发一次 MCP 调用（确认 daemon 启动）
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | nc -U ~/.local/share/arshy/arshyd.sock

# 3. 等待 10 秒（空闲超时）
# 4. 确认 daemon 进程退出：pgrep arshyd 无输出
# 5. 再发一次调用
echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | nc -U ~/.local/share/arshy/arshyd.sock
# 6. 确认 daemon 自动重启并正常响应
```

## 不做什么

- 不改变 MCP tool schema（保持 2-tool 模型）
- 不新增 IPC 方法（daemon/shutdown 已存在）
- 不修改 daemon 核心逻辑（只改 proxy 空闲监控）
- 不支持 Windows 预编译二进制（首版）
- 不内置沙箱/容器隔离（用户自行配置）
- 不修改插件市场注册流程（先手动安装验证，再申请收录）
