# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-04

arshy 的初始公开版本 — AI Agent 的结构化执行层。

- **2-tool MCP 模型**：`arshy_exec`（执行/管理任务）+ `arshy_query`（跨任务全历史事件查询）。
- **Auto-mode 智能**：短命令零开销直出，长命令结构化解析 + 异步通知 + 原文输出通道。
- **37 个内置 parser**：声明式 TOML，可热重载，带 49 组 fixture 测试。
- **安全沙箱**：命令过滤、路径沙箱、权限分级、审计日志。
- **零残余接入**：8 个一等 agent + 任意 MCP agent；`uninstall` 只删 arshy 写入的内容。
- **JSONL 执行记忆**：跨 session 可查询，按需自启 + 空闲退出。
