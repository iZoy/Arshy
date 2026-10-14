# Arshy 中文指南

Arshy 是面向 AI Agent 的本地命令执行与诊断层，通过标准 MCP 提供服务。
![原生 cargo 输出与 Arshy 结构化诊断对比](../assets/arshy-demo.gif)

同一个失败的 `cargo check`：原生方式下，Agent 需要通读完整编译输出自行定位错误；
经过 Arshy，Agent 直接拿到 status、exit_code、错误数和一个 primary_diagnostic
（错误码、文件、行、列），原始输出仍可按需获取。演示基于
[可复现示例](../tutorials/agent-diagnostic-loop.md)，可用 `scripts/record-demo.py`
重新生成。

对 Agent 的价值：

- 统一结构：38 个内置 parser 覆盖的构建、测试、lint、部署失败共用严重级别、位置和错误码字段；
- 减少重复阅读：重复诊断会被合并，汇总行不会被重复计为错误；
- 按需取证：需要时再通过 arshy_query、arshy_task 获取更多事件或原始输出。

v0.1.0-alpha.1 是公开预览版，接口可能在正式 0.1.0 前调整。当前支持 macOS
和 Linux。

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

公开预览阶段不宣称固定 token 节省比例。欢迎通过 GitHub Issues 提交安装问题、
解析错误或一次真实任务的使用反馈。
