# arshy — 开源发布清单

> 按真实产物重写（2026-08-03，v0.2.0）。**发布动作需用户明确批准后执行**——当前处于开源前冲刺，不部署 GitHub、不打 tag、不 release。
> 对应仓库：https://github.com/iZoy/Arshy（main 分支，本地领先 origin 50 个提交，未推送）

## 真实产物基线（v0.2.0 实测）

| 维度 | 真实数值 |
|------|---------|
| 二进制 | 双二进制：`arshy`（CLI + MCP proxy）+ `arshyd`（daemon） |
| MCP 模型 | 2 工具：`arshy_exec` + `arshy_query` |
| 内置 Parser | 37 个 TOML 声明式（tsc/cargo/jest/eslint/python/go/docker/kubectl/terraform…） |
| Parser 管线 | 6 级：格式检测 → Stateful → TOML → Crash → 启发式 → Raw；ReDoS 校验 |
| Parser fixture | 171 组（.txt 输入 + .json 期望） |
| Agent 接入 | 8 个 registry：claude-code / cursor / vscode / antigravity / codex / opencode / aider / workbuddy |
| 存储 | 纯 JSONL：`~/.local/share/arshy`（tasks.jsonl + events/ + raw/），无数据库依赖 |
| 安全 | 命令过滤 + 路径沙箱 + 权限分级 + 审计日志 + peer UID 校验（UDS 0600） |
| 自愈 | 请求驱动自动拉起 + 熔断（5 次/120s）+ spawn-lock + setsid 脱离会话 + 15min 空闲退出 |
| 测试 | 471 lib + 61 bin + 2 daemon 全绿；clippy `-D warnings` 零告警；fmt 通过 |
| Dogfooding | scripts/dogfood.sh 21/21（含 daemon 脱离会话回归检查）；`--report` 产出指标快照；CI 门禁同套 |
| 报告 | `dogfood.sh --report [--report-json <path>]` 一命令产出版本/store/stats/analyze 快照 |

## Pre-Launch 检查（按真实功能核对）

### 功能
- [x] 一键安装：`install.sh [--agent <id>] [--dry-run]`（构建或下载 + codesign + hook + setup + doctor）
- [x] 一行接入：`curl ... | sh -s -- --agent codex`
- [x] 零残余卸载：`arshy uninstall --agent <id>`（未知 id 报错并列合法值）
- [x] 短命令直出（零开销）、长命令结构化（summary/root_cause/±3行 context/git 关联）
- [x] 长任务 >120s 委托提示（`arshy query/kill`）
- [x] 8 agent 接入 + `arshy doctor --agent <id>` 校验
- [x] stats / analyze（内部，供 report）/ parser list/reload/benchmark / self-update
- [x] macOS TCC 兜底（/tmp/.arshy-cwd symlink）
- [x] `arshyd --version`（2026-08-03 补齐）

### 质量
- [x] 470 + 61 + 2 单元测试全绿
- [x] dogfood 21/21（本机 + CI）
- [x] clippy/fmt 零告警（CI 强制）
- [x] 1000 task 稳定性压测（2026-08-03，隔离 store 实测：1000 任务 0 失败/0 kill/0 timeout；
  1000 任务下 stats 响应 7ms、query 响应 4ms；store 8.4MB（1000 任务+2000 事件）；
  串行 160ms/任务，并行 8 路 273ms/任务——daemon 默认 4 并发槽位是批量场景瓶颈，agent 串行调用不受影响）
- [ ] 24h 连续运行验证（按需自启 + 15min 空闲退出设计下不适用常驻场景；发布前可选跑一轮）

### 安全
- [x] 命令执行在用户权限下
- [x] UDS peer UID 校验 + 0600 权限
- [x] parser 纯 TOML 声明式（无 Rhai 脚本，无任意代码执行面）
- [x] 命令过滤（curl|sh 等 blocked pattern 实测拦截）

### 跨平台
- [x] macOS（开发主平台，arm64 实测；x86_64 仅编译矩阵）
- [x] Linux（CI ubuntu-latest 全绿：fmt/clippy/test/doc/dogfood）
- [ ] Windows (WSL) — 未验证，发布文档标注 "macOS & Linux"

## Day 0：发布流程（获批后按序执行）

1. **推送 + 打 tag**
   - `git push origin main`（本地领先 50 提交）
   - `git tag v0.2.0 && git push origin v0.2.0` → 触发 release.yml
2. **验证 release 产物**：`arshy-v0.2.0-{aarch64-apple-darwin,x86_64-apple-darwin,x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu}.tar.gz` + sha256（install.sh 下载 URL 已对齐此命名，含 ARM64 Linux）
3. **无 cargo 机器实测一行安装**（release 前唯一未端到端验证的路径；本机已通过 install.sh --dry-run + release 构建冒烟）
4. **crates.io**：`cargo publish`（Cargo.toml 已含 license/repository/description/readme/exclude；`cargo package` 本地校验通过——crate 含全部 37 parser + fixtures + LICENSE，仓库专属文件已排除）
5. **Homebrew**：Formula/arshy.rb 已就绪（`brew install arshy` 走 cargo 构建）。发布产物生成后切换到 prebuilt tarball + sha256（公式内注释已给出模板），实现秒装
6. **MCP 目录注册**：smithery.ai（待做）
7. **README 第一屏**：已就绪（定位表 + 一行安装 + 8 agent 列表）
8. **证据快照**：发布前重跑 `scripts/evidence_snapshot.sh`，`docs/evidence/` 内的指标快照随 release 一起作为真实性证据

## Day 1+：推广（待发布后）

- [ ] Hacker News Show HN（对比 demo：raw bash vs arshy 结构化）
- [ ] Reddit r/rust / r/programming
- [ ] X / 开发者社区（Claude Code / Cursor Discord）
- [ ] 用 `dogfood.sh --report` 的指标快照作为"产品真实性"证据帖素材

## 回滚 / 应急

- 版本回退：改 Cargo.toml version → 重新 push tag
- 数据恢复：JSONL 纯文件，`tasks.jsonl` + `events/*.jsonl` + `raw/*` 直接拷贝即备份
- 用户卸载：`arshy uninstall --agent <id>` 或 `arshy uninstall`（零残余）

## 发布阻断项（已知，不阻塞本次冲刺之外的决策）

- 无 cargo 环境的一键安装依赖 GitHub Release 产物 → 必须 tag 后验证
- GitHub 大小写：repo 是 iZoy/Arshy，URL 大小写不敏感但文档需统一
- 本机 Codex 应用对 `~/.local/bin/arshy` 精确路径 SIGKILL（Taskgated）→ install.sh 的 ad-hoc codesign 是必需步骤，发布文档需注明"不要手动 cp 绕过"
