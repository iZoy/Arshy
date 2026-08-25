# Arshy 中文指南

Arshy 是通过标准 MCP 暴露的本地结构化命令执行与诊断层。当前是内部开发候选版
**v0.1.0-dev.1**，尚未正式公开发布。

安装脚本只下载、校验并放置 `arshy` 与 `arshyd`，不会修改 agent 配置、shell
启动文件、hook 或项目文件：

```sh
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh \
  | sh -s -- --version v0.1.0-dev.1
```

使用 `arshy mcp config` 获取通用 MCP 配置，然后粘贴到任意 MCP 客户端。项目
不再提供按 agent 定制的一键接入或卸载命令。

效率数据使用版本化的 quality-v1 分项指标，由内容收敛、噪声过滤、诊断完整度、
去重减少四项组成，不计算聚合分数。它是工程指标，不是 token 节省率；只在
显式调用 `stats`、`analyze` 或 benchmark 时出现。

目录限制配置为 `security.allowed_cwds`。这是 cwd 目录守卫，不是容器或操作系统
沙盒，命令仍以当前用户权限运行。
