# Arshy 路线图

## 当前状态（v0.1.0）

| 维度 | 指标 |
|------|------|
| 内置 Parser | 31 个，覆盖 ~85% 开发者常用工具 |
| 格式检测 | JSON/NDJSON/YAML/CSV/TSV |
| 智能输出 | 摘要 + 根因 + 上下文 |
| 测试 | 288 项全通过 |
| MCP | Smart Sync，1 次调用 = 完整结果 |
| 安全 | 命令过滤、socket 0600、审计日志 |

## 已完成

### 核心架构
- [x] Smart Sync Shell（60s 超时，1 次调用 = 完整结果）
- [x] 5 级 Parser 管道（格式检测→Stateful→TOML→Crash→Raw）
- [x] 格式检测（JSON/NDJSON/YAML/CSV/TSV）
- [x] 智能输出（summary + root_cause + project_context）
- [x] 链式命令检测（`&&`/`||`/`;` 分段匹配）

### Parser 生态
- [x] 31 个内置 Parser（tsc/cargo/eslint/python/go/npm/terraform/kubectl/helm/aws/docker/uv/ruff/turbo/nx/deno/bun 等）
- [x] TOML schema v1.0（deprecated/replaced_by/since_version）
- [x] ReDoS 安全校验
- [x] Parser 热重载 + diff 审计
- [x] ARSHY_BLESS=1 自动生成 fixture

### 工程质量
- [x] CI/CD（fmt/clippy/test/doc on PRs）
- [x] 集成测试（真实 daemon + proxy + MCP）
- [x] Socket 安全（0o600）
- [x] 进程树优雅关闭
- [x] 遥测计数器
- [x] 配置版本化 + 安全边界

### Agent 体验
- [x] MCP 自描述（权威 instructions）
- [x] Plugin 安装（`claude plugin install arshy`）
- [x] 只读检查工具短路径（40+ 工具瞬间返回）
- [x] 健康检查指数退避

## 下一步（P0）

### Parser 生态扩展
- [ ] 10+ 个新 Parser（svelte-check、pyright、yarn lint、pnpm lint、vite test 等）
- [ ] 每个 Parser 至少 3 个 fixture 测试
- [ ] 社区贡献流程文档

### 格式检测扩展
- [ ] 键值对格式（`key=value`）
- [ ] 表格格式（对齐列）
- [ ] 更多 JSON 变体（JSON5、JSONC）

### Agent 智能输出
- [ ] 错误模式数据库（常见错误 → 修复建议）
- [ ] 命令优化建议（检测慢命令，建议替代）
- [ ] 上下文丰富化（相关文件内容、git blame）

## 中期（P1）

### 分发
- [ ] 发布到 Claude Code marketplace
- [ ] 发布到 GitHub marketplace
- [ ] 英文文档

### 高级功能
- [ ] 多项目 Parser 配置（`.arshy/parsers/` 项目级）
- [ ] 命令缓存（避免重复执行）
- [ ] 性能分析（per-phase 耗时）

### 生态集成
- [ ] Cursor 集成
- [ ] GitHub Copilot 集成
- [ ] VS Code 扩展

## 长期（P2）

### 智能化
- [ ] 自动修复建议（错误模式匹配 + 修复方案）
- [ ] 命令意图识别（理解 agent 在做什么）
- [ ] 学习机制（从 agent 反馈中改进 parser）

### 安全
- [ ] 沙箱模式（seccomp/pledge）
- [ ] 审计日志可视化
- [ ] 命令风险评分

### 性能
- [ ] 流式输出（实时推送行输出）
- [ ] 并行任务管理
- [ ] 远程 daemon 支持

## 产品哲学

### 核心价值
arshy = 结构化信息提取器

当前：从命令**输出**中提取结构化信息（Parser）
未来：从命令**意图、结果、上下文**中提取有用信息

### 终极目标
Agent 跑一个命令，拿到的不只是"输出是什么"，而是"这意味着什么，下一步该做什么"。

### 护城河
Parser 生态 = arshy 的真正护城河

类比：
- Docker 的护城河是 Docker Hub 的镜像生态
- ESLint 的护城河是规则生态
- arshy 的护城河是 Parser 生态

每增加一个 Parser，arshy 的价值就增加一分。
