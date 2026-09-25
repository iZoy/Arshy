# Parser 参考

> 本文档依据 `parsers/builtin/*.toml`（38 个）、`src/daemon/parser/toml_def.rs`、`src/daemon/parser/registry.rs`、`src/daemon/parser/mod.rs` 核对（arshy v0.1.0-alpha.1）。内置 parser 经 `rust-embed` 编译进二进制，另有 `raw` fallback，因此运行时默认显示 39 个条目。

## 内置 Parser 清单

38 个内置 parser（`parsers/builtin/<name>.toml`）。检测匹配的是命令**前缀**（大小写不敏感，`starts_with`），路径式调用按 basename 匹配；`route = "fast"` 的资产只负责明确的只读检查快路径。

| 名称 | 类型 | priority | detect（首词前缀） | detect_full | 用途（description） | 模式数 |
|---|---|---|---|---|---|---|
| `aws` | toml | 50 | `aws` | — | AWS CLI | 5 |
| `biome` | toml | 55 | `biome` | — | Biome linter/formatter（formerly Rome） | 4 |
| `bun` | toml | 50 | `bun` | — | Bun JavaScript runtime | 8 |
| `cargo` | toml | 50 | `cargo`, `rustc` | — | Cargo build tool / rustc compiler | 7 |
| `cargo-test` | toml | 55 | — | `cargo test` | Cargo test runner | 11 |
| `cc` | toml | 50 | `gcc`, `g++`, `clang`, `clang++`, `cc` | — | C/C++ compilers | 5 |
| `clippy` | toml | 55 | `clippy` | — | Clippy Rust linter | 3 |
| `curl` | toml | 50 | `curl` | — | curl HTTP client（整 parser 已 deprecated） | 8 |
| `deno` | toml | 50 | `deno` | — | Deno JavaScript/TypeScript runtime | 7 |
| `docker` | toml | 50 | `docker` | — | Docker CLI | 14 |
| `esbuild` | toml | 50 | `esbuild` | — | esbuild bundler | 3 |
| `eslint` | toml | 50 | `eslint` | — | ESLint linter | 5 |
| `git` | toml | 55 | `git` | — | Git version control | 14 |
| `go` | toml | 50 | `go` | — | Go compiler and test runner | 4 |
| `gradle` | toml | 50 | `gradle`, `gradlew` | — | Gradle build tool | 4 |
| `helm` | toml | 50 | `helm` | — | Helm Kubernetes package manager | 5 |
| `inspection` | toml | 1000 | `git`, `ls`, `pwd`, `echo` 等 | `git status/log/diff/show` | 只读检查命令快路径路由（无结构化事件） | 0 |
| `jest` | toml | 50 | `jest`, `vitest` | — | Jest / Vitest test runner | 7 |
| `kubectl` | toml | 50 | `kubectl` | — | kubectl Kubernetes CLI | 6 |
| `make` | toml | 50 | `make` | — | GNU Make build tool | 4 |
| `mocha` | toml | 50 | `mocha` | — | Mocha test runner | 5 |
| `npm` | stateful | 50 | `npm` | — | npm package manager | 5 |
| `nx` | toml | 50 | `nx` | — | Nx monorepo build tool | 4 |
| `oxlint` | toml | 55 | `oxlint` | — | Oxc linter（Rust 实现的 JS linter） | 5 |
| `pip` | stateful | 50 | `pip`, `pip3` | — | pip package installer | 5 |
| `pnpm` | stateful | 50 | `pnpm` | — | pnpm package manager | 4 |
| `prettier` | toml | 50 | `prettier` | — | Prettier code formatter | 4 |
| `python` | toml | 50 | `python`, `python3`, `pytest` | — | Python / pytest | 9 |
| `ruff` | toml | 50 | `ruff` | — | Ruff Python linter | 4 |
| `ssh` | toml | 55 | `ssh`, `scp`, `rsync` | — | SSH remote access | 7 |
| `swc` | toml | 50 | `swc` | — | SWC compiler | 4 |
| `terraform` | toml | 50 | `terraform` | — | Terraform infrastructure tool | 5 |
| `tsc` | toml | 50 | `tsc` | — | TypeScript compiler | 4 |
| `turbo` | toml | 50 | `turbo` | — | Turborepo monorepo build tool | 4 |
| `uv` | toml | 50 | `uv` | — | uv Python package manager | 5 |
| `vite` | toml | 50 | `vite` | — | Vite build tool | 4 |
| `vitest` | toml | 55 | `vitest` | — | Vitest test runner | 6 |
| `webpack` | stateful | 50 | `webpack` | — | Webpack bundler | 5 |

另有恒存在的 `raw` fallback 条目（`ParserSource::Builtin`，priority 0，不参与检测，作为 6 层管线第 5 层兜底）。

### 状态与优先级要点

- `cargo test` 命中 `cargo-test`（detect_full，priority 55），`cargo build` 命中 `cargo`（priority 50）；
- `jest` 的 detect 含 `vitest`，但 `vitest` 独立 parser priority 55 更高，命令 `vitest ...` 先命中 `vitest`；
- 同 priority 时 user parser 覆盖 builtin（排序 `priority desc`，再 `source`：Builtin < User，dedup 保留首个）；
- 检测回退：命令含 `&&`/`||`/`;` 时按段拆分重试（如 `ls; python3 -m pytest -v` → `python`）；`rustc file.rs` 映射到 `cargo` parser。

## TOML Schema

定义文件示例（`parsers/builtin/tsc.toml`）：

```toml
[meta]
name = "tsc"
description = "TypeScript compiler"
detect = ["tsc"]
priority = 50

[[pattern]]
name = "ts-error"
regex = '^(.+?)\((\d+),(\d+)\): error (TS\d+): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, column = 3, code = 4, message = 5 }
```

反序列化结构：`TomlParserDef { meta: MetaDef, pattern: Vec<PatternDef> }`（顶层 `#[serde(default)]`，缺省字段取默认值）。解析失败（TOML 错误）的 parser 会被跳过并记录 error；正则非法或被 `redos::check_safe` 拒绝的模式被跳过并记录 warning。

### [meta] 字段

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `name` | string | `""` | parser 名称（注册表按此去重） |
| `description` | string | `""` | 用途描述 |
| `detect` | string 数组 | `[]` | 命令首词前缀匹配（如 `"cargo"` 匹配 `cargo build`） |
| `detect_full` | string 数组 | `[]` | 全命令前缀匹配（如 `"cargo test"` 匹配 `cargo test -- --test-threads=1`） |
| `parser_type` | string | `"toml"` | `"toml"`（stateless 行匹配）或 `"stateful"`（状态机跨行匹配） |
| `priority` | u32 | `50` | 检测优先级（大者先）；builtin 只用 50/55 |
| `min_version` | string | 无 | 工具最低版本（semver，含）；经 `detect::version_satisfies` 判断 |
| `max_version` | string | 无 | 工具最高版本（semver，含） |
| `schema_version` | string | `"1.0"` | 该 parser 编写时依据的 schema 版本（默认 `"1.0"`；`aws`/`docker`/`kubectl` 显式写 `"1.0"`，其余文件省略） |
| `since_version` | string | 无 | 首次加入时的版本（当前 38 个文件均未使用） |
| `deprecated` | bool | `false` | 整个 parser 弃用标记；加载时打印 warning |
| `replaced_by` | string | 无 | 弃用后的替代 parser/方案名 |

### [[pattern]] 字段

| 字段 | 类型 | 默认值 | 说明 |
|---|---|---|---|
| `name` | string | `""` | 模式名 |
| `regex` | string | `""` | 行匹配正则；经 `redos::check_safe` ReDoS 安全检查 |
| `event_type` | string | `""` | 事件类型（见"事件类型词汇"） |
| `severity` | string | `""` | 固定严重级别（`error`/`warning`/`info`）；配合 `fields.severity` 捕获组可动态取值 |
| `fields` | map<string, usize> | `{}` | 捕获组 → 事件字段映射（见下） |
| `state_condition` | string | 无 | stateful-only：`"key=value"`，仅当状态键等于该值时才匹配 |
| `state_transition` | string | 无 | stateful-only：`"key=value"`，匹配后设置状态 |
| `deprecated` | bool | `false` | 模式弃用标记 |
| `replaced_by` | string | 无 | 替代模式名（须存在于同一 parser） |
| `since_version` | string | 无 | 该模式引入版本（当前保留字段，未被使用） |

### fields 映射

`LinePattern` 转换只读取以下键（`toml_def.rs` to_line_pattern）：

| 键 | 捕获组含义 |
|---|---|
| `file` | 文件路径 |
| `line` | 行号（解析为 u64，失败为 0） |
| `column` | 列号（解析为 u64） |
| `code` | 错误码 |
| `message` | 消息文本（trim） |

特殊键 `severity`：指向一个捕获组时，事件 severity 取捕获文本归一化后的值
（小写前缀匹配 `error`/`warning`/`info`；`fatal`→error、`warn`→warning、
`note`/`debug`→info）；**识别失败时回退到 pattern 固定 `severity`（fail-safe）**。
eslint 的 `eslint-diagnostic`/`eslint-rule-id` 使用它，fixture 中 warning 行
被正确标为 `warning`。`cargo`/`ssh` 使用的 `duration_s`/`crate_name`/`version`/
`user`/`host`/`port` 等自定义键仍被忽略（仅 `message = 0` 形式的整行捕获生效）。

stateful 模式（`to_stateful_pattern`）只使用 `message`/`file`/`line` 捕获组。

### 事件类型词汇

内置 parser 使用的 `event_type`（按出现次数）：`diagnostic`（96）、`summary`（53）、`test_result`（21）、`location`（18）、`error`（12）、`warning`（2）、`progress`（2）、`success`（2）、`log`（2）、`data`（2）、`pod_status`（2）、`action`（1）。severity 值：`error`（107）、`info`（83）、`warning`（23）。

## stateful parser

`npm`、`pip`、`pnpm`、`webpack` 为 stateful（`parser_type = "stateful"`，无行模式）。状态机行为：

- `state_condition`/`state_transition` 格式为 `key=value`（`parse_key_value` 校验两段均非空，非法则忽略）；
- 模式按序喂入 `feed_line`，`state_transition` 在匹配后更新状态；后续行只有满足 `state_condition` 才匹配；
- 命令结束时 `on_complete(exit_code, seq)` 依据状态产生最终事件。

现有状态转换（`parsers/builtin/*.toml`）：

| parser | 模式 | state_transition |
|---|---|---|
| `npm` | `npm-packages-added` | `packages_added=done` |
| `npm` | `npm-error` | `has_error=true` |
| `pip` | `pip-installed` | `installed=done` |
| `pip` | `pip-error` | `has_error=true` |
| `pnpm` | `pnpm-added` | `packages_added=done` |
| `pnpm` | `pnpm-error` | `has_error=true` |
| `webpack` | `webpack-error` | `has_error=true` |

## 生命周期（deprecated / replaced_by）

当前状态（核对自 TOML 文件）：

| 位置 | 模式/parser | replaced_by |
|---|---|---|
| `curl` parser（meta） | 整 parser deprecated | `python-requests`（内置集中不存在该 parser） |
| `bun` → `bun-test-result` 模式 | deprecated | `bun-test-pass` |
| `curl` → `curl-http-error` 模式 | deprecated | `curl-http-status` |
| `deno` → `deno-test-result` 模式 | deprecated | `deno-test-pass` |

- 加载 deprecated parser 时打印 `builtin parser 'curl' is deprecated`（元数据不再指向不存在的替代 parser）；
- registry 记录每 parser 的 `deprecated_count`（当前 3 个模式），reload diff 会报告计数变化；
- 全部 38 个文件的 `since_version` 均为空；`schema_version` 统一为 `"1.0"`（`aws`/`docker`/`kubectl` 显式写出，其余缺省，代码默认 `"1.0"`）。

## 检测算法（`registry.rs` detect）

1. 命令小写化；取首词 basename（`./node_modules/.bin/tsc` → `tsc`）；
2. 按 priority 遍历注册表（raw 跳过），先尝试 `detect_full`（对全命令 `starts_with`），再尝试 `detect`（对首词 `starts_with`）；结构化 parser 优先于 `route = "fast"` 资产；
3. 命令含 `&&`/`||`/`;` 时按段拆分并重复检测，优先返回任一结构化命中，只有没有结构化命中时才返回 fast 路由；
4. 未命中返回 `None` → 会话退化为纯 raw/log 管线（crash + heuristic 仍生效）。

版本约束：`min_version`/`max_version` 存在时，对探测到的工具版本做 semver 含端点判断（`detect.rs` version_satisfies），不满足则不选用该 parser。

## 解析管线（6 层，`parser/mod.rs` ParserSession::parse_line）

| 层 | 模块 | 说明 |
|---|---|---|
| 1 | `json.rs` | 行级 JSON 解析（`try_parse_line`）；命中即返回 |
| 2 | `stateful.rs` | 状态机跨行匹配（命中返回，否则继续） |
| 3 | `toml.rs` | 行正则匹配（模式按序、首匹配胜出） |
| 4 | `crash.rs` | 通用崩溃/回溯检测（无工具依赖） |
| 4.5 | `heuristic.rs` | 关键词启发式错误过滤（无工具依赖） |
| 5 | `toml::raw_event` | raw fallback：每行一个 `log` 事件，按关键词分类 severity（error/warning/info） |

其他处理阶段（任务级，`src/daemon/exec/`）：`dedup.rs`（连续相同事件折叠）、`pair_merger.rs`（rustc 风格诊断配对合并）。执行路径不读取源码文件，也不关联 Git 变更。

## Fixture 约定

- 目录：`parsers/builtin/tests/<tool>/`（目录名对应 TOML 文件名：`docker.toml` → `tests/docker/`）；
- 每 fixture 一对文件：`<name>.txt`（输入）与 `<name>.json`（期望输出，TaskEvent 数组）；
- 当前共 **60 对 fixture**，38 个 parser 中有结构化输出的 parser 每者至少 1 对；`inspection` 是纯路由资产，不产生 parser 事件，因此不设 fixture；
- 生成/校验：`ARSHY_BLESS=1 cargo test fixture_` 自动生成期望 JSON；fixture 测试验证 ≥95% 字段准确率（type、severity、code、file、line）；
- 添加 parser 流程：新建 `parsers/builtin/<tool>.toml` → 建 fixture 目录与 `.txt` 输入 → bless 生成 `.json` → 验证测试通过。

## 与直觉不符的事实（代码核对）

1. `parser benchmark` 会把报告写入 `docs/benchmark_report.json`（docs 目录存在时），属于"命令写入文档目录"的副作用。
