# Arshy 中文指南

面向编程 Agent 的本地 CLI 诊断层（MCP）：把嘈杂的构建 / 测试 / lint 输出收成结构化证据（严重级别、文件、行号、错误码）。**作品集 / 公开预览**，不宣称「所有 Agent 必备基础设施」。

![原生 cargo 输出与 Arshy 结构化诊断对比](../assets/arshy-demo.gif)

同一个失败的 `cargo check`：原生方式下 Agent 通读编译器全文；经 Arshy 则拿到 status、exit_code 与 primary_diagnostic（错误码、文件、行、列），原始输出仍可按需取回。演示见 [可复现示例](../tutorials/agent-diagnostic-loop.md)（`scripts/record-demo.py`）。

## 诚实结论（Codex，2026-10-15）

在三道 **修到通过** 题（Rust 多错误、嘈杂 pytest、Go 多错误）上，原生 shell 与 Arshy 均为 **3/3** 成功。Arshy 平均工具轮次略少（4.7 vs 5.7）；总输入 token 接近（约 416k vs 426k）；墙钟时间更慢（约 14 分钟 vs 11 分钟，含 Codex 重连噪声）。仅诊断烟雾题上仍有明显的 MCP 宿主 / schema token 开销。**结论：** 可复现的原型，偶有轮次收益；尚未证明 token 或速度优势，也不是必备基础设施。

完整表格：[results-hard-fix-2026-10-15.md](../benchmarks/results-hard-fix-2026-10-15.md)。方法：[benchmarks/README.md](../benchmarks/README.md)。

## 它是什么

- 一个 MCP 服务（`arshy_exec` / `arshy_query` / `arshy_task`）加本地 daemon。
- 约 38 个内置 CLI parser，共用诊断字段，重复项会合并。
- 按需取证：需要时再取更多事件或原始输出。
- 已走通发布链路：musl 静态 Linux 包、校验和安装脚本、GitHub Release [v0.1.0-alpha.1](https://github.com/iZoy/Arshy/releases/tag/v0.1.0-alpha.1)。

v0.1.0-alpha.1 接口仍可能调整。支持 macOS 与 Linux；Linux 为 musl 静态链接（Ubuntu 22.04 / Debian 12 可用）。

## 安装

~~~bash
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/v0.1.0-alpha.1/install/install.sh \
  | bash -s -- --version v0.1.0-alpha.1
arshy daemon start
arshy doctor
~~~

安装器只下载并校验 arshy 与 arshyd，不修改 Agent 配置、shell 启动文件、
hook 或项目文件。

## 接入

在 Agent 将要工作的项目根目录执行：

~~~bash
arshy mcp config --format prompt
~~~

把完整输出交给 Agent，让它使用客户端原生 MCP 管理方式注册。注册成功不代表
当前会话已经加载工具；请检查配置，然后重启客户端或开启新会话，确认出现
arshy_exec、arshy_query 与 arshy_task。如果已有同名但内容不同的配置，先解决
冲突，不要直接覆盖。

结构化结果位于 MCP structuredContent，并同时提供文本回退。需要更多诊断时
使用 arshy_query，需要查看捕获文本时使用 arshy_task 的 raw 操作。

## 数据、安全与反馈

Arshy 以当前用户权限执行命令，不是操作系统或容器沙盒。
security.allowed_cwds 只检查工作目录。任务历史保留在配置的数据目录，删除
二进制不会自动删除这些数据。

arshy doctor --format json 只在本地生成版本、系统架构、daemon 与协议检查状态、
解析器数量；不会自动上传。提交 Issue 前请自行审阅。

定位为**作品集 / 公开预览**：早期 Codex 基准未显示相对原生 shell 的明确 token 或速度优势，详见上文诚实结论。不宣称固定节省比例。欢迎通过 GitHub Issues 反馈安装问题、解析错误或真实任务体验。
