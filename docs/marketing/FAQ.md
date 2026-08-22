# 开发者常见问题（FAQ）

> 面向**想用 arshy 的开发者**：安装、集成、性能、安全、卸载等问题。

---

## 安装与平台

### Q: arshy 支持哪些系统？
**A**: 仅 **Unix-like**（macOS / Linux）。依赖 Unix Domain Socket、进程组信号、`sh -c` 管道。**Windows 原生不在支持范围，暂无 WSL 支持计划**（WSL2 内属 Linux 环境，按 Linux 对待）。

### Q: 怎么安装？
**A**: 一行命令接入 Codex：
```bash
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh -s -- --agent codex
```
其他 agent 见 [`docs/tutorials/setup-agent.md`](../tutorials/setup-agent.md)。

### Q: 接入后能完全卸载吗？会留垃圾吗？
**A**: `uninstall` 只删 arshy 写入的内容——AGENTS.md 段、`.mcp.json` 中的 arshy 项、`~/.arshy/` 数据目录。**不碰**用户的其他配置。

---

## 性能与开销

### Q: 短命令（`echo hi`、`ls`）有开销吗？
**A**: 零开销直出。`is_short_command()` 判定命中后直接返回 raw stdout，不入库、不解析、不发事件。这是 hot path。

### Q: 长命令（`cargo test`、`npm install`）的开销有多大？
**A**: 实测：

| 阶段 | 开销 |
|---|---|
| 解析管道 | ~5-50ms（取决于输出行数） |
| 上下文提取（±3 行） | async，不阻塞主流程 |
| 持久化（JSONL） | async，写盘不阻塞 |
| 总响应延迟（sync mode） | ~10-100ms（取决于 daemon 唤醒 + 解析） |

### Q: arshy 比 RTK / Headroom 节省更多 token 吗？
**A**: **我们没有跑 head-to-head benchmark**。架构差异：
- RTK/Headroom 是**后处理层**（命令跑完后压缩文本）
- arshy 是**执行层**（在执行期重组成结构化）

实测 token 节省率：6 个主流生态 **+45.7% 到 +79.9%**（measured，0 fallback）。详见 [`docs/marketing/CLAIMS.md`](CLAIMS.md)。

### Q: 节省率怎么算的？可信吗？
**A**: 见 [`docs/reference/metrics.md`](../reference/metrics.md)。关键：
- 每个 task 有 `savings_basis` 字段（`measured` / `estimated` / `none`）
- 只有 `measured` 才能引用
- 10% 兜底已显式标记（之前会静默放大数字）

### Q: daemon 内存占用？
**A**: ~22 MB（dogfooding 数据 758 tasks 后）。Daemon 默认空闲 60 秒退出。

---

## 集成

### Q: 我用的 Agent 不在支持列表怎么办？
**A**: arshy 提供标准 MCP 协议（stdio JSON-RPC），任何支持 MCP 的 agent 都能用——`arshy init` 在项目根写 `.mcp.json`，agent 自动发现。

### Q: 接入会影响我现有的 CI / shell 脚本吗？
**A**: 不影响。`arshy run "..."` 是显式调用；普通 bash 脚本继续用 bash。除非你主动接入了 hook，否则 agent 不会自动走 arshy。

### Q: `arshy run` 和直接调 `arshy_exec`（MCP 工具）有什么区别？
**A**: 
- `arshy run "cmd"` — CLI 调用，等同于 agent 在 shell 里手动执行
- `arshy_exec` (MCP) — Agent 在循环里调，带结构化响应

两者背后走同一个 daemon / 同一个 store。

---

## 行为细节

### Q: 怎么知道哪些命令走了 arshy？哪些走 raw？
**A**: 
```bash
arshy status              # 看 daemon 状态
arshy list --limit 10    # 看最近 10 个 task
```
短命令不会出现在 `list` 里（它们直接返回 raw stdout，没入库）。长命令才会。

### Q: 长命令超时怎么办？
**A**: 默认 60 秒（sync mode 自动超时退到 async；async mode 立即返回）。可用 `--timeout-ms N` 自定义：
```bash
arshy run "cargo build" --timeout-ms 300000   # 5 分钟
```

### Q: 怎么取消正在跑的 task？
**A**:
```bash
arshy list                            # 找 task_id
arshy kill <task_id>                  # graceful → force
```

### Q: 重启 daemon 会丢失历史吗？
**A**: 不会。所有 task + events 持久化到 `~/.local/share/arshy/store/`（JSONL）。Daemon 重启后 `arshy list` / `arshy query` 仍能查询。Daemon 空闲超时（默认 60s）也会自动退出但数据保留。

### Q: 怎么查看具体 task 的事件？
**A**:
```bash
# 列表
arshy list --limit 5

# 详情（含事件）
arshy query <task_id>

# 原始输出
arshy tail <task_id> --lines 50
```

---

## 安全与权限

### Q: arshy 会在我的系统上做什么？
**A**: 
- 装在 `~/.local/bin/arshy` 和 `~/.local/bin/arshyd`
- 数据目录：`~/.local/share/arshy/`
- 配置目录：`~/.arshy/`
- Socket：`~/.local/share/arshy/arshyd.sock`

**不会**：写其他位置、装 cron、注册 systemd service。

### Q: 沙箱能阻止危险命令吗？
**A**: 默认 `none` 模式只审计日志（passive），不阻止。`workspace` 模式会限制文件系统访问：
```bash
ARSHY_DAEMON_SANDBOX_MODE=workspace arshy daemon start
```
或运行时配置（见 `docs/reference/config.md`）。

### Q: 我的命令会被上传到任何地方吗？
**A**: **不会**。所有处理本地完成。没有 telemetry、没有外发请求。详见 `docs/reference/config.md`。

### Q: 接入时改了我的 `.bashrc` / `.zshrc` 吗？
**A**: 默认**不会**。`arshy init` 写的是项目级 `.mcp.json` 和 AGENTS.md，不是 shell rc。如果你显式选了 shell 集成方案（CLAUDE.md hook 等），会在项目目录下写文件，**不修改全局 shell 配置**。

---

## 故障排除

### Q: `arshy: daemon did not start within 10s`
**A**: 99% 是端口/socket 冲突。试试：
```bash
ls -la ~/.local/share/arshy/arshyd.sock
# 如果存在但无进程在跑：
rm ~/.local/share/arshy/arshyd.sock
# 或：
scripts/restart.sh
```

### Q: Agent 不调 `arshy_exec`？
**A**: 三个常见原因：
1. **没重启 agent**——MCP 配置改了需要重启
2. **`.mcp.json` 路径不对**——`arshy doctor --agent codex` 检查
3. **agent 不认 MCP**——少数老 agent 可能不认；fallback 用 `arshy run` CLI

### Q: 我看到 `savings_basis: "estimated"` 该担心吗？
**A**: 意思是至少有 1 个 task 用了 10% 兜底启发式（不是真实测量）。原因通常是：
- Task 跑得太快没经过 enrichment 路径
- 历史 task 没记录 delivered_bytes

**怎么排查**：`daemon/stats` 里有 `savings_fallback_task_count` 看具体多少个。短期可接受，长期应让所有 task 走 enrichment 路径。

### Q: 卸载后数据还在吗？
**A**: `uninstall` 默认**保留** `~/.arshy/` 和 `~/.local/share/arshy/` 数据。要彻底删除：
```bash
uninstall --agent codex --purge    # 删所有 arshy 写入的内容 + 数据
```

---

## 仍有问题？

- 文档索引：`docs/index.md`
- Issue：https://github.com/iZoy/Arshy/issues
- 性能声明复现：`scripts/measure-savings.sh`
