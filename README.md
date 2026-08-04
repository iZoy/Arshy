# Arshy

**AI Agent 的结构化执行层（structured execution layer）。** Agent 跑命令拿结构化结果——`file:line` 定位、错误码、源码上下文；短命令零开销直出、长命令结构化解析、历史可查询、按需自启 + 空闲退出、卸载零残余。

| | Raw Bash | Arshy |
|-|----------|-------|
| 输出 | 非结构化文本 | 结构化事件（error/warning/file/line） |
| 错误定位 | Agent 自己 grep | 精确 file:line:col + 源码上下文 |
| 长任务 | 阻塞或超时 | 异步 + 实时通知 + 优雅 kill |
| 安全 | 无 | 命令过滤 + 路径沙箱 + 权限分级 + 审计 |
| 历史 | 无 | JSONL 跨 session 查询（执行记忆） |

## 快速开始

```bash
# 一行安装并接入 Codex（macOS / Linux，支持 8 个 agent）
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh -s -- --agent codex

# 或已装好的情况下，每个 agent 一条命令
arshy setup codex        # 接入一个 agent
arshy doctor --agent codex   # 验证
arshy run "echo hello world" # 第一条命令（短命令零开销直出）
```

**核心特性**

- **2-tool MCP 模型**：`arshy_exec`（执行/管理任务）+ `arshy_query`（查询事件，支持跨任务全历史搜索）。
- **原文输出通道**：`arshy_exec(action:"raw", task_id)` 随时取回命令原始输出（`lines=0` 取全部）；长命令零结构化事件时，响应自动提示该通道——Agent 需要原文时不再"失明"。
- **Auto-mode 智能**：短命令原样秒回；长命令（cargo build、npm test）结构化解析 + 异步通知。
- **37 个内置 parser**：tsc/cargo/jest/eslint/go/python/docker/kubectl/terraform 等，声明式 TOML，可热重载、可社区贡献。
- **安全沙箱**：命令过滤、路径沙箱、权限分级、审计日志（默认开启）。
- **零残余接入**：8 个一等 agent + 任意 MCP agent（`arshy init` 三层协议路径）；`uninstall` 只删 arshy 写入的内容。

## 项目结构

```
src/            Rust 源码（双二进制 + 库）：cli/（命令）、proxy/（MCP 代理）、daemon/（守护进程）、ipc/（协议）、mcp/（工具定义）、config/（配置）
parsers/builtin/  37 个内置解析器的 TOML 定义 + tests/<tool>/ fixture
docs/          文档（Diátaxis 四象限 + 归档 + 指标证据快照）
scripts/       dogfood 自测 / 证据快照 / 数据回填等开发脚本
arshy/         Claude Code 插件打包（plugin.json + hooks + skills）
install/       一键安装脚本；Formula/ 为 Homebrew 配方
```

## 文档

文档按 Diátaxis 框架组织（Tutorials / How-to / Reference / Explanation），另附产品与设计档案：

- [文档首页](docs/index.md)
- [安装教程](docs/tutorials/install.md) · [第一条命令](docs/tutorials/first-command.md) · [接入 Agent](docs/tutorials/setup-agent.md)
- [配置](docs/how-to/configure.md) · [自定义 Parser](docs/how-to/create-parser.md) · [测试体系](docs/how-to/run-tests.md)
- [CLI 参考](docs/reference/cli.md) · [MCP 协议](docs/reference/mcp.md) · [IPC 协议](docs/reference/ipc.md) · [Parser 参考](docs/reference/parsers.md)
- [架构](docs/explanation/architecture.md) · [解析管线](docs/explanation/parser-pipeline.md) · [设计原则](docs/explanation/design-principles.md)

## 许可证

MIT
