# 设计原则：克制、语义、深度、可扩展

arshy 的大部分设计决策可以从一份原则清单推导出来。这些原则不是写在某个设计文档里的口号，而是可以直接在代码中读到的判断：MCP 指令文本的措辞、响应里的字段取舍、解析器注释里的"not advice"、以及 renderer 的"只用于人类观察"的自我约束。

本文按重要性展开五条核心原则，最后给出一个明确的不做清单——在 arshy 的语境里，"不做"往往和"做"同样重要。

## 原则一：克制主义（token 克制三准则）

arshy 面向的读者是 LLM，而 LLM 的上下文是有限资源。因此**默认少给，需要时再给**成为第一原则。它落实为三条可验证的准则：

### 准则 1：默认摘要，lazy pull

长命令的 MCP 响应不是"完整输出"，而是一行摘要：

```
✗ 3 errors, 10.5s (exit 1)
Root cause: cannot find value `undefined_var` in this scope
Changed files:
 src/main.rs | 2 +-
```

（对应 `src/proxy/handlers.rs` 中"Long command → concise structured summary"的注释——agent 拿到的默认是状态图标、耗时、错误数、失败时的根因与变更文件。）

全量事件**不随响应发送**：内联事件按结果裁剪（失败 ≤20 条 error，成功 ≤5 条 warning/info），超出部分返回 `events_truncated` 与 `events_hint`（"Call arshy_query(task_id:...) for the rest"）。`arshy_query` 就是 lazy pull 的通道：想看单任务详情、跨任务搜索执行记忆、按 severity/code/file 过滤，都是按需查询。MCP 指令文本甚至直接告诉 agent："Don't manage task IDs or poll."——默认路径不需要任何额外动作。

工具定义本身也遵守这条准则：MCP 只暴露两个工具（`arshy_exec` + `arshy_query`），注释写明"2-tool model ... Reduces ~60% tool definition tokens and improves agent selection accuracy"。工具定义是每次握手都要传输的固定开销，工具越少，上下文越省。

### 准则 2：不重复 LLM 常识

arshy **不合成 cause/fix/retry 提示**。曾经存在的 HintDb（错误码 → 修复建议映射）被整体移除，执行器注释给出理由：

> arshy's job is structured extraction (file:line location, error code, severity, source context), **not** advice. Synthesising cause/fix hints is the LLM's task.

静态的"错误码 → 建议"表有两个结构性缺陷：它假设错误与修复存在稳定映射（实际取决于项目上下文），且它把过期建议包装成结构化事实（LLM 会倾向信任）。修复一个编译错误对 LLM 来说是常识，arshy 重复这份常识只会稀释结构化事实的密度。`TaskEvent.hint` 保留为 null 兼容占位，协议留槽、语义不填。

受限参考层（2026-08 决策 2）与此不同：它只解释**非显而易见**退出码的含义（docker 125/126/127/137、kubectl/aws 退出码），通过 `arshy_query` 按需返回，是参考数据而非建议——"代码是什么"归 arshy，"怎么办"仍归 LLM。见 [错误码参考表](../reference/reference-codes.md)。

### 准则 3：只暴露本地事实

响应里的每一个增强字段都来自"命令输出 + 本地仓库状态"，不掺任何外部知识：

- `root_cause` 来自输出事件本身（第一条 error 事件，traceback 场景取最后一条诊断错误）；
- `project_context` 来自本地 `git diff --stat HEAD~1` 与错误文件关联（`correlated_errors`）；
- ±3 行源码上下文来自本地文件读取。

没有网络查询、没有模型推理、没有"我们猜你可能想……"。克制在这里同时是诚实：**结构化的价值来自可验证性**，一旦掺入猜测，字段就失去了可信度。

## 原则二：语义 > 压缩

"少给 token"很容易被误读成"压缩文本"。arshy 的选择恰恰相反：**优先保留语义，压缩发生在语义提取之后**。

- 对长命令，输出给 agent 的不是压缩后的原文，而是 `file:line: code: message` 的结构化事件——一条 `error[E0425]: cannot find value ...` + `location` + `context` 比截断的原始输出更短，但信息更完整；
- benchmark 同时度量两个指标：`compression_ratio`（raw token / 结构化 token）和 `fields_per_event` / 带 location/code 的事件比例。前者是压缩效率，后者是语义保真——两者被设计为**一起看**，因为"压得很小但丢了位置信息"是失败而不是成功；
- 短命令的"直通原始输出"同样是语义优先：对 `ls`、`git diff` 这类命令，原始文本就是最高保真的语义，强行结构化反而丢失信息。

## 原则三：深度 > 广度

解析器数量（37 个）是结果，不是目标。真正的投入在每个解析器的深度上：

- **状态机**：npm、webpack 这类跨行块状输出有 `state_condition`/`state_transition` 的状态机，而不是一叠互不相关的正则；
- **后处理**：rustc 上下文行吸收、diagnostic+location 配对合并——这些是为"单个错误在数据里是一个事件"服务的深度机制；
- **覆盖目标**：70% TOML / 25% stateful / 5% crash/raw 的比例假设"常用工具的输出是稳定的"，所以值得为它们做深；通用层（crash/heuristic）只做够用的浅覆盖。

广度上的克制同样明显：通用启发式层只有一组精心防误报的关键字模式（`error handling` 不得误报为错误），而不是堆砌"看起来能抓到更多错误"的宽松正则。深而准，胜过广而噪。

## 原则四：可扩展 > 硬编码

工具输出格式是持续变化的外部世界。arshy 的对策是让"认识一种新格式"变成数据变更而不是代码变更：

- 解析器是 TOML 数据文件（`parsers/builtin/*.toml`），新工具 = 新文件 + fixture，不需要碰 Rust；
- 用户解析器（`~/.arshy/parsers/`）可覆盖内置同名解析器，`priority` 排序、同名去重；
- 热重载（notify 文件监听）让解析器变更即时生效，无需重启守护进程；
- 模式生命周期显式化：`deprecated = true` + `replaced_by` + `since_version`，禁止不告而删；
- schema 版本化（`schema_version`）保护向后兼容；
- 代理层预留 `Middleware` 链（request/response/notification 三向钩子），MCP 协议版本支持四个稳定版本并回显客户端版本。

数据驱动的同时还有护栏：TOML 正则加载时过 ReDoS 静态校验（拒绝嵌套量词、重叠分支+重复），保证"可扩展"不会变成"可被自己的正则打垮"。

## 原则五：安全默认，非可选

安全不是开关，而是执行路径的固有部分（详见 security-model）。最有力的证据在短路径上：`run_short` 跳过了 store、解析器与事件总线——这三个都被视为"可选增强"——但**限速、命令过滤、路径沙箱与审计一条都不跳过**。也就是说，性能优化可以牺牲结构化，不能牺牲安全。

另一条证据是配置校验：`sandbox_mode` 只接受 `none` 或 `workspace`，守护进程启动时校验，非法值直接拒绝启动（fail-closed，而不是 warn 后放行）。

## 数据驱动：用 fixture 与 benchmark 说话

设计判断要被量化验证，而不是靠口头辩论：

- 49 组 fixture（每个解析器的 `.txt` 输入 + `.json` 期望输出）锁死解析行为，`ARSHY_BLESS=1` 可重新生成期望；
- 内置 benchmark 报告压缩比、字段密度、错误定位速度（结构化事件是否比人工在原始输出里找更快）、未解析错误行数；
- 去重折叠数、配对合并数、git 关联错误数记入任务计数器，管线质量是运行时观测值。

## 不做清单

克制主义的另一半是明确拒绝一些"看起来顺理成章"的方向：

| 不做 | 为什么 |
|---|---|
| **ML 压缩输出** | 用模型压缩输出是在结构化层引入不确定性与额外延迟；结构化提取是确定性的，可验证、可测试、可 benchmark |
| **把 bash 变成 API** | 不发明"命令对象"或强制参数化——命令就是字符串，通过 `sh -c` 执行。透明 bash 代理（见 integration-model）说明 arshy 甚至不要求调用方改变行为 |
| **人用 shell** | CLI 默认输出 JSON（agent-first），renderer 模块自我声明"只用于人类观察工具：stats、benchmark、analyze"。`arshy run --format pretty` 只服务显式的人类观察场景 |
| **强制接管** | 拦截只在满足条件时发生：工作区显式 opt-in（`.arshy.toml`）、或父进程是白名单内的 agent、且无 TTY 冲突、且非递归调用。未知进程、人类终端、`ARSHY_BYPASS=1` 一律透明放行（见 integration-model） |
| **合成 cause/fix 建议** | 见准则 2：建议是 LLM 的职责，静态 hint 表会被移出（也确实被移出了） |

## 原则如何协作：一个失败构建的例子

`cargo build` 失败时，五条原则同时作用：

1. **克制**：响应是一行摘要 + 根因 + 变更文件，最多 20 条 error 事件内联，其余提示去 `arshy_query`（准则 1）；没有任何"你可能忘了什么"的建议（准则 2）；`git diff --stat` 来自本地仓库（准则 3）；
2. **语义**：`E0425` + `file:line:column` + 前后 3 行源码，而不是截断的编译器输出；
3. **深度**：rustc 的 `-->` 位置行与 `= note:` 上下文行被合并进诊断事件，而不是散落成碎片；
4. **可扩展**：这个工具的输出格式由 `parsers/builtin/cargo.toml` 数据驱动，热重载可调；
5. **安全**：过滤、限速、审计全程在线。

这五个决定共同回答了"为什么 arshy 长这样"：它把命令执行从"给 LLM 一段文本"变成"给 LLM 一组关于本地世界的、可验证的结构化事实"，并且只在这个边界内做事。
