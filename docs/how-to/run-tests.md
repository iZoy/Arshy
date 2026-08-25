# 如何运行与扩展测试

arshy 的测试体系分四层：**单元测试 → 集成测试（真实进程）→ Parser fixture → Dogfood 回归**。
每层回答不同问题，全部通过才允许提交（见 [测试体系](../explanation/testing.md)）。

## 快速运行全部测试

```bash
cargo build --bin arshy --bin arshyd     # 集成测试需要真实二进制
cargo test --lib --bin arshy --bin arshyd # 单元测试（480+66+2）
cargo test --test integration             # 集成测试（真实 daemon + MCP proxy 进程）
cargo test --bin arshyd fixture_cargo     # 单个 parser fixture 示例
ARSHY=./target/debug/arshy ./scripts/dogfood.sh  # 21 项 dogfood 回归
```

提交前完整门禁（与 CI 一致）：

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --lib --bin arshy --bin arshyd
cargo test --test integration
cargo doc --no-deps --document-private-items
ARSHY=./target/debug/arshy ./scripts/dogfood.sh
```

## 单元测试

分布在 `src/**/*.rs` 的 `#[cfg(test)]` 模块内，覆盖 parser、store、exec、IPC、
proxy、analytics、config、security 各层。运行：

```bash
cargo test --lib --bin arshy --bin arshyd
# 只跑某个模块/用例
cargo test --lib store::tests::search
cargo test --bin arshy proxy::tests::test_initialize_echoes_newer_supported_version
```

## 集成测试（真实进程）

`tests/integration.rs` 会真实拉起 `arshyd` daemon 与 `arshy mcp serve` proxy
子进程，走完整 UDS/JSON-RPC 与 MCP 链路。每个测试使用**独立临时目录**（socket +
store），`TestDaemon` 守卫保证进程必被回收，不会污染真实数据或挂起 cargo。

```bash
cargo test --test integration          # 全部（macOS / Linux 均可运行）
cargo test --test integration run_echo # 单个用例
```

覆盖范围：daemon health、短命令直出、长命令结构化、跨任务搜索、list/tail、
kill、安全拦截、优雅关闭、MCP 协议版本协商、MCP 工具调用全链路。

新增集成测试的要点：

- 用 `TestDaemon::spawn()` 起隔离 daemon，用 `daemon.rpc(id, method, params)` 发请求；
- 读响应会**自动跳过 notification 行**，按 `id` 匹配响应；
- 不要依赖"上一行就是响应"的假设（daemon 会先推送事件通知）；
- 不要继承 stdout/stderr 管道给子进程（否则挂起 `cargo test`）。

## Parser fixture 测试

每个内置 parser 在 `parsers/builtin/tests/<tool>/` 下至少有一组
`.txt`（输入）与 `.json`（期望事件）fixture。fixture 测试由
`src/daemon/parser/mod.rs` 的 `run_parser_fixtures` 生成并强制校验字段匹配率。

```bash
cargo test --bin arshyd fixture_<tool>   # 例如 fixture_cargo / fixture_python
```

**新增/更新 fixture（bless 流程）**：

1. 新建或修改 `parsers/builtin/tests/<tool>/<name>.txt`（真实工具输出）；
2. 运行 `ARSHY_BLESS=1 cargo test --bin arshyd` 自动生成期望 `.json`；
3. 人工检查生成的 `.json` 符合预期（severity/code/file/line 字段）；
4. 正常跑 `cargo test --bin arshyd` 确认通过（字段匹配率 ≥95%）。

## Dogfood 回归

`scripts/dogfood.sh` 让 arshy 用自己跑自己：daemon 健康、短/长命令、错误提取
（E0308 + 源码上下文）、git 关联、安全拦截、去重、stats/analyze，共 21 项。

```bash
ARSHY=./target/debug/arshy ./scripts/dogfood.sh
# 带报告（含指标快照）
ARSHY=./target/debug/arshy ./scripts/dogfood.sh --report
```

dogfood 所有命令自动打 `--purpose dogfood` 标签，不会污染"真实开发"失败率口径。

## CI 门禁

`.github/workflows/ci.yml` 在 push/PR 时依次执行：fmt check → clippy
`-D warnings` → 单元测试 → doc build → 集成测试 → dogfood。全部通过才算绿。
