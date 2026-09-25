# 为任意 CLI 工具编写自定义 TOML parser

本文解决以下任务：为某个 CLI 工具编写 TOML parser（普通/stateful 两种 pattern 类型）、处理 deprecated/replaced_by 生命周期、用 fixture 测试与 `ARSHY_BLESS` 验证、用 `parser reload` 热加载验证。

parser 定义是 TOML 文件：38 个 builtin 定义在 `parsers/builtin/*.toml`（编译时嵌入，`src/daemon/parser/toml_def.rs` 的 `BuiltinAssets`），用户定义放在 `parser.dirs` 配置的目录（默认 `~/.arshy/parsers`）。加载/匹配逻辑见 `src/daemon/parser/`（`registry.rs`、`toml.rs`、`stateful.rs`、`mod.rs`）。

## 1. 决定 parser 文件放哪里

| 场景 | 位置 | 加载方式 |
|---|---|---|
| 本机使用（不提交） | `~/.arshy/parsers/<tool>.toml` | daemon 启动时从文件系统加载；`hot_reload` 默认开启，改动自动生效 |
| 贡献给 arshy 仓库 | `parsers/builtin/<tool>.toml` + `parsers/builtin/tests/<tool>/` fixture | 编译时嵌入，需跑 fixture 测试 |

两个来源会合并（`ParserRegistry::load`）：

- 排序：priority 高者优先；priority 相同时，**builtin 排在用户 parser 之前**（`src/daemon/parser/registry.rs` 的排序实现是 `a.source.cmp(&b.source)`，`Builtin < User`；代码注释声称 "user beats builtin"，与实际行为不符——以实测为准）；
- 同名去重：按排序后的顺序先到先得。因此**要用同名文件覆盖 builtin parser，必须把 priority 设得更高**（builtin 默认多为 50，例如设 60）；同 priority 时 builtin 会赢；
- 用户 TOML 若没有 `detect` 列表，默认以**文件名（不含扩展名）**作为 detect 词。

## 2. TOML schema

完整 schema 见 `src/daemon/parser/toml_def.rs`，字段全部可选（除语义上必须的 `name`/`regex` 等）。

### [meta] 段落

```toml
[meta]
name = "mytool"            # parser 名，唯一
description = "..."        # 说明
detect = ["mytool"]        # 按命令第一个词匹配（starts_with）
detect_full = ["mytool check"]  # 按完整命令匹配（多词命令，如 "cargo test"）
parser_type = "toml"       # "toml"（默认）| "stateful"
priority = 50              # 匹配优先级，默认 50
# min_version / max_version: 工具版本上下限（可选；TOML 不支持 null，省略即不限制）
schema_version = "1.0"     # 本文件针对的 schema 版本，默认 "1.0"
since_version = "0.1.0"    # 本 parser 首次引入的版本（新增时填写）
deprecated = false         # 整个 parser 弃用
replaced_by = "other-tool" # 弃用时的替代 parser 名
```

### [[pattern]] 段落

```toml
[[pattern]]
name = "my-error"                  # pattern 名（parser 内建议唯一）
regex = '^(.+?):(\d+): error (E\d+): (.+)$'
event_type = "diagnostic"          # 任意字符串，建议 diagnostic/summary/log/data
severity = "error"                 # error|warning|info 等
fields = { file = 1, line = 2, code = 3, message = 4 }  # 捕获组索引 → 事件字段
deprecated = false                 # 本 pattern 弃用
replaced_by = "my-error-v2"        # 替代 pattern（须在同一 parser 内）
since_version = "0.1.0"            # 本 pattern 引入版本
```

`fields` 支持的键与含义（`LinePattern`，`src/daemon/parser/toml.rs`）：

| 键 | 含义 |
|---|---|
| `file` | 文件路径（生成 `location.file`） |
| `line` | 行号（解析为 u64，失败则 0） |
| `column` | 列号 |
| `code` | 错误码（如 `TS2322`） |
| `message` | 事件消息（匹配后 trim；缺省时整行） |
| `severity` | 特殊键：指向某个捕获组，用其值**覆盖**静态 `severity`（归一化为 `error`/`warning`/`info`；识别失败回退静态值，fail-safe） |

捕获组 `0` 表示整个匹配（builtin 中常见，如 `fields = { message = 0 }`）。

#### 动态 severity 示例

工具把等级印在文本里时（如 ESLint 的 ` 1:10  error  ...` / ` 22:10  warning  ...`），
可以用一个 pattern 通吃两种等级：

```toml
[[pattern]]
name = "lint-line"
regex = '^\s+(\d+):(\d+)\s+(error|warning)\s+(.+?)\s+(\S+)$'
event_type = "diagnostic"
severity = "error"                  # 回退值：捕获组文本识别失败时使用
fields = { line = 1, column = 2, severity = 3, message = 4, code = 5 }
```

`error` 行 → `severity: error`；`warning` 行 → `severity: warning`；
若捕获组文本不是已知等级（如 `fatal`/`warn`/`note`/`debug` 已归一化，其他
未知文本则回退到 `severity = "error"`）——动态能力永远不会比固定值更糟。
eslint 的 `eslint-diagnostic` / `eslint-rule-id` 即用此特性（fixture 覆盖
warning 与 error 两种等级）。

## 3. 普通（stateless）pattern

普通 parser 逐行匹配：每一行按 pattern 顺序尝试，**第一个命中的 pattern 生效**（`TomlParser::parse_line`）。

参考 builtin `parsers/builtin/tsc.toml`：

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

匹配示例：`src/app.ts(10,5): error TS2322: ...` → `{ file: "src/app.ts", line: 10, column: 5, code: "TS2322", severity: "error", event_type: "diagnostic" }`。

## 4. stateful pattern（跨行状态机）

工具输出跨多行（npm 错误块、webpack chunk 等）时用 `parser_type = "stateful"`。stateful parser 维护 `key=value` 状态表（`StatefulParser`，`src/daemon/parser/stateful.rs`）：

- `state_condition = "key=value"`：仅当状态表中 `key == value` 时才匹配；
- `state_transition = "key=value"`：匹配后把 `key` 设为 `value`；
- 命令结束时调用 `on_complete(exit_code)` 可补发最终事件（例如超时/失败汇总）。

参考 builtin `parsers/builtin/npm.toml`：

```toml
[meta]
name = "npm"
description = "npm package manager"
detect = ["npm"]
parser_type = "stateful"
priority = 50

[[pattern]]
name = "npm-packages-added"
regex = '^added (\d+) packages?'
event_type = "summary"
severity = "info"
state_transition = "packages_added=done"

[[pattern]]
name = "npm-error"
regex = '^npm ERR! (.+)'
event_type = "diagnostic"
severity = "error"
fields = { message = 1 }
state_transition = "has_error=true"
```

注意：stateful parser 只有 `message`/`file`/`line` 三个字段映射（无 `column`/`code`）。

## 5. 匹配管线与工具识别

每行输出按固定管线处理（`ParserSession::parse_line`，`src/daemon/parser/mod.rs`）：

```
1. JSON 行检测  → 2. Stateful  → 3. TOML 正则 → 4. Crash 检测 → 5. Heuristic 过滤 → 6. Raw 兜底
```

- 前 5 层都命中时才返回事件；全不命中则 `raw_event` 按关键词（error/failed/panic/traceback…）分类 severity。
- 工具识别（`ParserRegistry::detect`）：
  1. `detect_full` 对完整命令做 `starts_with`（如 `cargo test` 命中 `cargo test -- --test-threads=1`）；
  2. `detect` 对第一个词做 `starts_with`（`cargo` 命中 `cargo build`）；
  3. 路径式调用按 basename 匹配（`./node_modules/.bin/tsc` → `tsc`）；
  4. 链式命令（`&&`/`||`/`;`）会逐段重试。
- 版本约束（`meta.min_version`/`max_version`）配合工具版本探测（`detect.rs` 的 `version_satisfies`）过滤 parser。

## 6. 编写完整示例（可复制）

假设工具 `mytool check` 输出：

```
src/main.rs:12: error E1001: undefined variable
src/main.rs:20: warning W2002: unused import
Checked 2 files, 1 error
```

### 6.1 本机自定义 parser

```bash
mkdir -p ~/.arshy/parsers
```

`~/.arshy/parsers/mytool.toml`：

```toml
[meta]
name = "mytool"
description = "Example CLI checker"
detect = ["mytool"]
detect_full = ["mytool check"]
priority = 60
schema_version = "1.0"
since_version = "0.1.0"

[[pattern]]
name = "my-error"
regex = '^(.+?):(\d+): error (E\d+): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, code = 3, message = 4 }
since_version = "0.1.0"

[[pattern]]
name = "my-warning"
regex = '^(.+?):(\d+): warning (W\d+): (.+)$'
event_type = "diagnostic"
severity = "warning"
fields = { file = 1, line = 2, code = 3, message = 4 }

[[pattern]]
name = "my-summary"
regex = '^Checked (\d+) files, (\d+) errors?$'
event_type = "summary"
severity = "info"
fields = { message = 0 }
```

验证（daemon 需在运行）：

```bash
arshy parser reload
arshy run "mytool check" --format json
```

### 6.2 带生命周期的版本化 pattern

同一 regex 语义变化时，不要删除旧 pattern，改为弃用并指到新名字：

```toml
[[pattern]]
name = "my-error-v1"
regex = '^(.+?):(\d+): ERROR: (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, message = 3 }
deprecated = true
replaced_by = "my-error"

[[pattern]]
name = "my-error"
regex = '^(.+?):(\d+): error (E\d+): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, code = 3, message = 4 }
since_version = "0.1.0"
```

项目 parser 约定另见[贡献指南](../../CONTRIBUTING.md)：

- 新增 pattern 必须带 `since_version`；
- 弃用必须用 `deprecated = true` + `replaced_by`（pattern 级替代必须在同一 parser 内）；
- 不允许无弃用周期地删除 pattern；
- 修改 TOML schema 时，在受影响的 parser 文件里 bump `schema_version`。

### 6.3 状态机示例

多行错误块（错误头 + 后续缩进行都属于同一诊断）：

```toml
[meta]
name = "blocky"
description = "Block-oriented tool"
detect = ["blocky"]
parser_type = "stateful"
priority = 50

[[pattern]]
name = "block-start"
regex = '^ERROR \[(.+?)\]'
event_type = "diagnostic"
severity = "error"
fields = { message = 1 }
state_transition = "in_error=yes"

[[pattern]]
name = "block-detail"
regex = '^\s+at (.+)$'
event_type = "log"
severity = "error"
state_condition = "in_error=yes"
fields = { message = 1 }
```

## 7. 用 fixture 测试验证（贡献 builtin parser 时）

每个 builtin parser 在 `parsers/builtin/tests/<tool>/` 下有 `.txt`（输入）与 `.json`（期望事件）成对文件（当前 60 对）。测试驱动逻辑在 `src/daemon/parser/mod.rs` 的 `harness_tests::run_parser_fixtures`。

### 7.1 创建 fixture

1. 新建 `parsers/builtin/tests/mytool/example.txt`，粘贴真实工具输出（每行一个样例，空行会被跳过）；
2. 新建测试入口：在 `src/daemon/parser/mod.rs` 的 `harness_tests` 模块中加：

   ```rust
   #[test]
   fn fixture_mytool() {
       run_parser_fixtures("mytool");
   }
   ```

3. 用 bless 模式自动生成期望 JSON：

```bash
ARSHY_BLESS=1 cargo test fixture_mytool -- --nocapture
```

期望输出（stderr）：`BLESSED: parsers/builtin/tests/mytool/example.json (N events)`。bless 模式只写 JSON、跳过断言（`run_fixture` 返回 total=0）。

> 注意：fixture 测试位于**库**测试（`arshy_lib`）里，用 `cargo test fixture_<name>` 运行；`cargo test --bin arshyd fixture_...` 只跑二进制自身的测试，会得到 `0 passed`。

### 7.2 跑 fixture 断言

```bash
cargo test fixture_mytool -- --nocapture
```

评分逻辑（>= 95% 才算通过）：

- 每个期望事件逐字段比对 `type`、`severity`、`code`、`file`、`line`；
- 全部字段一致计为 matched；`matched / total >= 0.95`，否则断言失败并打印分项百分比：

```
parser 'mytool' fixture 'example': 100% (3 / 3) [type=100% sev=100% code=100% file=100% line=100%]
```

- 期望 JSON 字段可省略（如无 code 的事件不写 `code` 键，该字段视为通过）；
- 事件顺序按 `.txt` 行序；`run_fixture` 会先经过 pair merger（诊断 + 上下文行合并），期望 JSON 与执行管线输出保持一致。

全部 fixture 一起跑：

```bash
cargo test fixture_ -- --nocapture     # 匹配所有 fixture_* 测试
```

## 8. 热重载与 reload 验证

- **热重载**：`parser.hot_reload = true`（默认）时，daemon 通过平台原生文件事件监视 `parser.dirs` 下所有 `.toml` 文件（非递归、500ms 去抖），增删改自动重建 registry（`src/daemon/parser/loader.rs`）；
- **手动重载**：`arshy parser reload` 重新从磁盘加载并打印 diff（新增 `+`、删除 `-`、pattern 数量/弃用数变化）。

```bash
arshy parser reload
```

实测输出（无变化时）：

```
  ✓ Parsers reloaded — changes detected:

    no changes
```

> 说明：registry diff 无变化时返回字符串 `"no changes"`（非空），所以 CLI 走"有变化"分支并打印黄色 `no changes` 行——这是当前版本的实际输出。有变化时看到的是 `+1 parsers: mytool` 之类带 `+`/`-` 前缀的行。

## 9. 常见失败与检查清单

- **regex 写错**：非法正则的 pattern 被跳过并在 daemon 日志打警告（`skipping pattern 'x': invalid regex ...`），其余 pattern 正常；
- **ReDoS 风险**：嵌套量词（`(?:a+)+`）或重叠分支 + 量词（`(a|ab)+`）会被 `redos::check_safe` 拒绝（`src/daemon/parser/redos.rs`），同样跳过并告警；
- **detect 不生效**：检查 `detect` 是否与命令第一词一致；多词命令用 `detect_full`；路径式调用靠 basename 匹配，不要在前面加路径；
- **fixture 找不到**：`run_parser_fixtures` 要求 `parsers/builtin/tests/<tool>/` 目录存在且至少有 1 个 `.txt`，否则断言 `no fixtures in ...`；
- **reload 看不到变化**：确认文件在 `parser.dirs`（默认 `~/.arshy/parsers`）且扩展名为 `.toml`（watcher 只关心 `.toml`）。
