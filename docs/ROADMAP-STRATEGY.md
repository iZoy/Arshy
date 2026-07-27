# arshy 战略路线图与北极星

**版本:** 1.3
**日期:** 2026-06-25
**状态:** 发布就绪

---

## 北极星

> **让 AI Agent 从命令输出中获得比人类开发者更多的信息。**

人类开发者看 `cargo build` 的 200 行输出，靠经验知道：E0308 是类型错误、大概率在最近改的文件里、试试 `.into()`。arshy 让 Agent 也拥有这个能力。

不是让 Agent 少读点（那是 RTK/Headroom 做的），而是让 Agent **读懂了**。

### 衡量标准

**主指标：Agent 从"看到错误"到"提交修复"的时间。**

这不是我们能直接测量的，但可以通过代理指标追踪：

| 代理指标 | 衡量方式 | 当前值 | 目标 |
|---------|---------|--------|------|
| Agent 可见事件率 | `arshy analyze` 中 agent_visible_events / total | **71.8%** | ≥ 80% |
| Token 节省率 | `arshy analyze` 中 noise_pct | **28.2%** | ≥ 50% |
| 事件准确率 | Fixture 测试字段匹配率 | **100%** | ≥ 95% |
| Context 丰富率 | 有 context 的 error 事件 | **175/853** | ≥ 90% |
| Parser 覆盖率 | `arshy stats` 中 parser_coverage_pct | **71%** | ≥ 80% |
| Daemon 内存 | RSS (758 任务) | **21.6 MB** | ≤ 30 MB |
| MCP 启动延迟 | 首次工具调用时间 | **4ms** | ≤ 100ms |
| 任务总数 | Dogfooding 积累 | **758** | 持续增长 |

---

## 市场定位

### 竞品格局

| 工具 | Stars | 定位 | 核心差异 |
|------|-------|------|---------|
| **RTK** | 61K | 输出过滤器 | Hook 拦截，100+ 硬编码过滤器，14 个 AI 工具 |
| **Headroom** | 22K | 通用压缩层 | ML 模型，可逆压缩，跨 agent 记忆 |
| **arshy** | — | 结构化执行层 | PTY 执行，6 层解析管道，语义提取 |

### arshy 的独特位置

**没有直接竞品做 arshy 在做的事。**

```
RTK/Headroom:  Agent → Bash → 原始输出 → 过滤/压缩 → 精简文本 → Agent
arshy:         Agent → arshy(MCP) → PTY执行 → 6层解析 → 结构化事件 → Agent
```

RTK 和 Headroom 是**后处理层**——在命令执行后压缩文本。
arshy 是**执行层**——控制命令的整个生命周期。

### 不竞争的领域

| 领域 | 为什么不做 |
|------|-----------|
| ML 压缩 | Headroom 已经做了，且需要大量训练数据 |
| 通用文本压缩 | 不是我们的核心价值 |
| 14 个 AI 工具的 hook 支持 | RTK 已经建立生态，追赶成本高 |
| 跨 agent 记忆共享 | Headroom 的方向，与我们的执行层定位不同 |

### 要竞争的领域

| 领域 | 为什么要做 |
|------|-----------|
| 结构化语义提取 | 我们的核心价值，没人做 |
| 多 agent 支持 | RTK 的 14 工具覆盖是护城河，我们必须跟上 |
| 解析器生态 | TOML DSL 是独特优势，要让用户能贡献 parser |
| 安全审计 | 没有竞品有这个，企业客户需要 |

---

## 路线图

### Phase 1: 基础设施 ✅ (已完成)

| 里程碑 | 状态 |
|--------|------|
| MCP server + 结构化输出 | ✅ |
| 37 个内置 parser | ✅ |
| 安全沙箱 (24 条规则) | ✅ |
| 限速器 | ✅ |
| SQLite 存储 + 事件流 | ✅ |
| 6 层解析管道 | ✅ |

### Phase 2: 智能层 ✅ (已完成)

| 里程碑 | 状态 |
|--------|------|
| 源码上下文 (±3 行) | ✅ |
| Git 变更关联 | ✅ |
| 错误码修复建议 (7 个精选) | ✅ |
| 启发式错误过滤器 | ✅ |
| 事件去重 | ✅ |
| 失败恢复 (tee) | ✅ |
| Log 事件过滤 (include_logs 参数) | ✅ |
| 增强遥测 (8 个 per-task 指标) | ✅ |
| 分析模块 (arshy analyze) | ✅ |
| 异步任务 Enrichment (所有任务) | ✅ |
| Context 噪音行过滤 | ✅ |
| RustcContextMerger 合并 | ✅ |
| Parser 优化 (噪音/测试/cargo/git/curl) | ✅ |
| 工具检测修复 (cd && tool) | ✅ |
| 9 个 code review bug 修复 | ✅ |
| 9 个 parser 审计 bug 修复 | ✅ |
| 性能优化 (脏标记/内存/IPC 稳定性) | ✅ |
| 7 个 code review bug 修复 | ✅ |

### Phase 3: 生态扩展 (当前)

**目标：让 arshy 从"能用"变成"有人用"。**

| 里程碑 | 优先级 | 状态 |
|--------|--------|------|
| 多 agent 支持 (Cursor, Codex, Copilot, Gemini) | P0 | ✅ Cursor |
| Hook 拦截模式 (零配置透明接入) | P0 | ✅ |
| 发布 v0.2.0 (GitHub Release) | P0 | ✅ 已发 |
| Dogfooding (永久实践：所有开发命令走 arshy，积累 stats 数据) | P0 | ✅ 进行中 |
| 社区发布 (README 完善 + 社区推广) | P1 | ✅ README 重写 |
| 收集 10 个用户的反馈 | P1 | ⬜ |
| **发布清单** | | |
| README 重写 (面向新用户) | P0 | ✅ |
| 安装文档 (macOS/Linux/Windows WSL) | P0 | ✅ |
| arshy doctor 命令 (诊断集成状态) | P0 | ✅ |
| 错误处理 (daemon 崩溃友好提示) | P0 | ✅ |
| MCP 自动配置 (Cursor 支持) | P0 | ✅ |
| MCP Prompt 模板 (3 个 prompt) | P0 | ✅ |
| MCP Resource (task output 暴露) | P0 | ✅ |
| 错误输出美化 (成功/失败差异化) | P1 | ✅ |

**成功标准：**
- 至少 3 个非 Claude Code 的 AI 工具能用 arshy
- 至少 10 个外部用户尝试过
- 收集到具体的用户反馈（不是猜测）

**不做：**
- 不加新 parser（等用户反馈）
- 不做 ML 压缩（不是我们的方向）
- 不做跨 agent 记忆（不是我们的方向）
- 不做人用的 shell wrapper（arshy 是 agent 的 shell）

### Phase 3.5: 性能与质量 ✅ (已完成)

**目标：生产级性能、数据质量、代码健壮性。**

| 里程碑 | 状态 |
|--------|------|
| Daemon 内存优化 (30.3MB → 21.6MB, -29%) | ✅ |
| 脏标记 + 定时刷新 (消除全量 JSONL 重写) | ✅ |
| raw_output 移出内存 (按需磁盘读取) | ✅ |
| EventBus 扩容 (256 → 4096 + 溢出警告) | ✅ |
| 重连通知保护 (排空旧通道) | ✅ |
| IPC 通道背压修复 (try_send 非阻塞) | ✅ |
| merge_enriched_events TOCTOU 竞态修复 | ✅ |
| 异步 enrichment 完整 (detected_tool 传递) | ✅ |
| sync/async enrichment 竞态修复 | ✅ |
| isError 语义修正 (exit>=2 才报错) | ✅ |
| Parser pattern deprecation cycle | ✅ |
| metrics 计算逻辑去重 | ✅ |
| Context 噪音行过滤 | ✅ |
| 37 个 parser 全量审计 + 9 bug 修复 | ✅ |
| 9 个 code review bug 修复 | ✅ |
| 7 个 code review bug 修复 | ✅ |
| 分析模块 (arshy analyze) + Pretty 输出 | ✅ |
| 增强遥测 (8 个 per-task 指标) | ✅ |
| 异步任务 Enrichment (所有任务) | ✅ |
| 迁移脚本 (历史数据重新处理) | ✅ |
| Agent 可见事件: 53.2% → 71.8% (+18.6%) | ✅ |

### Phase 4: 护城河 (数据驱动)

**目标：基于 Phase 3 的用户反馈，建立不可替代的优势。**

| 方向 | 触发条件 | 说明 |
|------|---------|------|
| 解析器生态 | 用户请求特定工具的 parser | 让用户能贡献 TOML parser，建立社区 |
| 跨命令因果分析 | 用户反馈 "Agent 不知道上一条命令影响了下一条" | 追踪命令历史，关联因果 |
| 智能重试 | 收集到真实的瞬态故障场景 | 基于错误类型建议重试策略 |
| 企业安全审计 | 有企业客户需求 | 完整的审计日志 + 合规报告 |
| 分析仪表板 | 用户想看 token 节省量 | `arshy analyze` 增强 + Web 仪表板 |
| 查询缓存 | query_events 频繁查询性能 | 内存缓存热点事件文件 |
| Post-processor 管道 | 新增 Python/Go context 合并 | 抽象为可插拔管道，替代内联链 |

**成功标准：**
- 用户驱动的功能迭代（不是我们猜的）
- 至少 1 个功能是用户反馈直接导致的
- Parser 覆盖率 ≥ 80%

**不做：**
- 不做 ML 压缩
- 不做通用文本压缩
- 不做跨 agent 记忆共享
- 不做 CLI 替代品（arshy 是 agent 的 shell，不是人的 shell）
- 不做人用的 shell wrapper

---

## 工程原则

### 做什么

| 原则 | 说明 |
|------|------|
| **语义 > 压缩** | 提取结构化信息比压缩文本更有价值 |
| **深度 > 广度** | 对主流工具的深度解析比覆盖 100 个工具的浅层过滤更重要 |
| **可扩展 > 硬编码** | TOML DSL 让用户能自定义，而不是硬编码 100 个过滤器 |
| **安全默认** | 命令过滤、路径沙箱、审计日志是默认行为，不是可选功能 |
| **数据驱动** | 新功能基于用户反馈，不是猜测 |

### 不做什么

| 原则 | 说明 |
|------|------|
| **不做 ML 压缩** | Headroom 已经做了，且需要训练数据 |
| **不做通用文本压缩** | 不是我们的核心价值 |
| **不做 CLI 替代品** | arshy 是 agent 的 shell，不是人的 shell。不做人用的 shell wrapper |
| **不做跨 agent 记忆** | Headroom 的方向，与我们的定位不同 |
| **不过度工程** | 只做用户需要的，不做我们觉得酷的 |

---

## 竞争策略

### vs RTK

| 维度 | RTK | arshy | 策略 |
|------|-----|-------|------|
| 广度 | 100+ 过滤器，14 工具 | 37 parser，1 工具 | 不追广度，追深度 |
| 深度 | 压缩文本 | 结构化语义 | **这是我们的优势** |
| 接入 | Hook 零配置 | MCP 需配置 | 补 hook 拦截模式 |
| 社区 | 61K stars，Discord | 无 | 先做好产品，再建社区 |
| 性能 | 文本压缩 | 21.6MB 内存，4ms 启动 | **生产级性能** |
| 数据 | 无 | `arshy analyze` 分析 | **数据驱动决策** |

**核心策略：不要跟 RTK 比谁覆盖的工具多，要比谁理解得深。**

### vs Headroom

| 维度 | Headroom | arshy | 策略 |
|------|---------|-------|------|
| 技术 | ML 模型压缩 | 正则/脚本解析 | 不同赛道，不竞争 |
| 范围 | 所有上下文 | 命令输出 | 专注 |
| 可逆性 | CCR 可逆压缩 | 不可逆 | 可能借鉴，但不是优先级 |
| Token 节省 | 未知 | 28.2% | **可量化** |

**核心策略：Headroom 压缩文本，arshy 提取语义。互补，不竞争。**

---

## 风险与缓解

| 风险 | 严重度 | 缓解措施 |
|------|--------|---------|
| 零用户，无法验证假设 | **高** | Phase 3 的核心任务：发布 + 收集反馈 |
| RTK 增加 MCP 支持 | **中** | 我们的深度解析是护城河，RTK 的硬编码过滤器无法复制 |
| MCP 生态萎缩 | **低** | MCP 是 Anthropic 主推标准，短期不会萎缩 |
| Agent 自己变强，不需要 arshy | **中** | Agent 变强是趋势，但结构化输出比原始文本永远更有价值 |
| 性能回归 | **低** | 脏标记 + 定时刷新机制，403 测试覆盖 |
| 数据一致性 | **低** | merge_enriched_events TOCTOU 修复，原子操作 |
| Parser 覆盖率不足 | **中** | 37 parser 全量审计，持续优化 |
| 异步任务 enrichment 不完整 | **低** | detected_tool 传递已修复，HintDb 可用 |

---

## 总结

**arshy 的技术差异化已经建立。** 37 个 parser、6 层管道、源码上下文、Git 关联——这些是竞品没有的。

**发布清单全部完成：**
- README 重写 (80 行，面向新用户)
- 安装文档 (macOS/Linux/Windows WSL)
- `arshy doctor` 命令 (诊断集成状态)
- 错误处理 (daemon 崩溃友好提示)
- MCP 自动配置 (Cursor 支持)
- MCP Prompt 模板 (3 个 prompt)
- MCP Resource (task output 暴露)
- 错误输出美化 (成功/失败差异化)

**Phase 3.5 性能与质量已全面完成：**
- Agent 可见事件率从 53.2% 提升到 71.8%
- Daemon 内存从 30.3MB 降到 21.6MB
- 37 个 parser 全量审计并修复 9 个 bug
- 22+ 个 code review bug 修复
- 性能优化：脏标记、内存优化、IPC 稳定性

**Dogfooding 已成为永久实践：**
- 所有开发命令通过 MCP 走 arshy（CLAUDE.md 强制要求）
- `arshy stats` 积累数据用于分析 parser 覆盖率、错误检测率
- `scripts/dogfood.sh` 作为提交前回归验证工具（21/21 通过）
- 不做人用 shell wrapper——arshy 是 agent 的 shell，不是人的 shell

**下一步：发布 v0.2.0，开始收集用户反馈。**

**北极星：让 AI Agent 从命令输出中获得比人类开发者更多的信息。**
