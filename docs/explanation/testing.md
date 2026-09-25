# 测试体系

arshy 的测试体系回答四个不同的问题，每一层都有明确的"测试对象"与"失败含义"：

| 层 | 测试对象 | 失败含义 | 位置 |
|----|---------|---------|------|
| 单元测试 | 单个模块/函数（parser、store、IPC、proxy 逻辑…） | 某个具体逻辑错了 | `src/**/*.rs` 内 `#[cfg(test)]` |
| 集成测试 | 真实 daemon + proxy 子进程的全链路 | 进程间契约/生命周期错了 | `tests/integration.rs` |
| Parser fixture | 解析器对真实工具输出的语义提取 | 某个 parser 提取的字段不对 | `parsers/builtin/tests/<tool>/` |
| Dogfood 回归 | arshy 自举：用自己跑自己的端到端体验 | 产品级体验退化 | `scripts/dogfood.sh` |

## 为什么这样分层

- **单元测试**是地基：无须启动完整客户端链路，可精确定位 parser、存储、IPC
  分发和代理响应整形等模块中的问题。
- **集成测试**补单元测试够不到的地方：真实 UDS 连接、daemon 生命周期、MCP
  握手与通知交错、跨进程超时与回收。每个 daemon 使用隔离临时目录，不触碰真实
  store。
- **Parser fixture** 固定解析器对代表性命令输出的结果（`ARSHY_BLESS=1` 生成期望、
  ≥95% 字段匹配率门禁，并要求实际事件数量与期望一致），避免规则变化时悄悄改变事件字段。
- **Dogfood** 是端到端回归：用本地构建的 Arshy 执行代表性开发命令，检查实际
  命令执行、安全拦截、诊断提取和分析报告路径；它不推导 token 节省率。

## 覆盖率口径

- **Parser**：结构化 parser 由 fixture harness 覆盖；`inspection` 仅负责路由，不
  产生 parser 事件。`run_parser_fixtures` 会检查已声明 fixture 的输入和期望输出。
- **进程链路**：集成测试覆盖 daemon health、短/长命令、查询（含跨任务搜索）、
  list/tail、kill、安全拦截、优雅关闭、MCP 协议协商与工具调用。
- **CI**：格式、Clippy、workspace 测试、文档构建、覆盖率门禁、打包审查和 dogfood。

## 已知边界（诚实声明）

- 集成测试未覆盖 GUI 级接入（如实际启动 Cursor/Codex 桌面应用）——那属于
  手动验收范围（见 `docs/tutorials/setup-agent.md` 的 doctor 验证流程）。
- 未做模糊/属性测试；覆盖率有全仓与关键路径门禁，parser 字段匹配率由 fixture 校验，
  但"事件准确率"的完整闭环依赖 fixture 语料持续扩充。
- 解析器对真实工具新版本的漂移风险靠 dogfood + 用户反馈兜底，不是自动化保证。

## 相关文档

- 如何运行与新增测试：[how-to/run-tests.md](../how-to/run-tests.md)
- 如何编写 parser 与 fixture：[how-to/create-parser.md](../how-to/create-parser.md)
- 解析管线的内部结构：[parser-pipeline.md](parser-pipeline.md)
