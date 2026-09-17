# 测试体系

arshy 的测试体系回答四个不同的问题，每一层都有明确的"测试对象"与"失败含义"：

| 层 | 测试对象 | 失败含义 | 位置 |
|----|---------|---------|------|
| 单元测试 | 单个模块/函数（parser、store、IPC、proxy 逻辑…） | 某个具体逻辑错了 | `src/**/*.rs` 内 `#[cfg(test)]` |
| 集成测试 | 真实 daemon + proxy 子进程的全链路 | 进程间契约/生命周期错了 | `tests/integration.rs` |
| Parser fixture | 解析器对真实工具输出的语义提取 | 某个 parser 提取的字段不对 | `parsers/builtin/tests/<tool>/` |
| Dogfood 回归 | arshy 自举：用自己跑自己的端到端体验 | 产品级体验退化 | `scripts/dogfood.sh` |

## 为什么这样分层

- **单元测试**是地基：毫秒级、无进程、可精确指向失败模块。arshy 的 500+
  单测覆盖了解析管线、存储、IPC 分发、代理响应整形等核心逻辑。
- **集成测试**补单元测试够不到的地方：真实 UDS 连接、daemon 生命周期、MCP
  握手与通知交错、跨进程超时与回收。历史教训：早期集成测试假设"一行即响应"、
  且子进程继承管道，导致在 macOS 上挂起后被整体 `#[ignore]`——测试"存在"但
  从未运行。现在它们跨平台运行并纳入 CI，且每个 daemon 使用隔离临时目录，
  绝不触碰真实 store。
- **Parser fixture** 是产品价值的直接度量：parser 是"把命令输出翻译成结构化
  事件"的翻译器，fixture 固定翻译结果（`ARSHY_BLESS=1` 生成期望、≥95% 字段
  匹配率门禁），并强制每个内置 parser 至少有一组 fixture——覆盖率不是口号。
- **Dogfood** 是哲学级的回归：arshy 声称"Agent 跑命令拿结构化结果"，那就用
  arshy 自己跑开发命令验证这条路径每天可用，同时积累真实的 quality-v1
  分项指标、解析覆盖率和可复现的失败证据；不推导 token 节省率。

## 覆盖率口径

- **Parser**：38 个内置 TOML 资产中，37 个结构化 parser 由 harness 覆盖 60 组
  fixture；`inspection` 是纯路由资产，不产生 parser 事件，因此没有 fixture。
  `run_parser_fixtures` 在声明了 fixture 的 parser 缺失输入时直接断言失败。
- **进程链路**：集成测试覆盖 daemon health、短/长命令、查询（含跨任务搜索）、
  list/tail、kill、安全拦截、优雅关闭、MCP 协议协商与工具调用。
- **CI**：fmt、clippy、单元、doc、集成、dogfood 六道门禁，缺一不可。

## 已知边界（诚实声明）

- 集成测试未覆盖 GUI 级接入（如实际启动 Cursor/Codex 桌面应用）——那属于
  手动验收范围（见 `docs/tutorials/setup-agent.md` 的 doctor 验证流程）。
- 未做模糊/属性测试与覆盖率百分比门禁；parser 字段匹配率由 fixture 校验，
  但"事件准确率"的完整闭环依赖 fixture 语料持续扩充。
- 解析器对真实工具新版本的漂移风险靠 dogfood + 用户反馈兜底，不是自动化保证。

## 相关文档

- 如何运行与新增测试：[how-to/run-tests.md](../how-to/run-tests.md)
- 如何编写 parser 与 fixture：[how-to/create-parser.md](../how-to/create-parser.md)
- 解析管线的内部结构：[parser-pipeline.md](parser-pipeline.md)
