# 安装 arshy

本教程带你从零安装 arshy，并在安装完成后验证一切可用。完成本教程后，你会得到两个可执行文件（`arshy` 与 `arshyd`）、一条可运行的 `arshy run` 命令，以及一个通过 `arshy doctor` 体检的安装环境。

## 前置条件

- 操作系统：macOS（Apple Silicon 或 Intel）或 Linux（x86_64 或 ARM64）。Windows 原生不支持；WSL2 内运行属于 Linux 环境，按 Linux 支持。
- shell：`bash`；下载预编译包时还需要 `curl` 或 `wget` 之一。
- 可选：Rust 工具链（`cargo`）。安装脚本检测到 `cargo` 时会优先从源码编译，否则下载预编译包。

arshy 由两个二进制组成：`arshy`（CLI/proxy）和 `arshyd`（后台 daemon）。安装脚本会把两个文件一起放进 `~/.local/bin`。

## 第 1 步：确认环境

先确认你的平台和是否有 Rust 工具链：

```bash
uname -s
uname -m
command -v cargo && cargo --version || echo "cargo not found"
```

- macOS：`uname -s` 输出 `Darwin`。
- Linux（包括 WSL2）：`uname -s` 输出 `Linux`。
- 有 `cargo`：安装脚本会执行 `cargo build --release` 编译；没有则下载预编译包。

## 第 2 步：一行安装

运行官方安装脚本：

```bash
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh
```

脚本会依次完成：

1. 创建安装目录 `~/.local/bin`。
2. 构建或下载二进制：
   - 检测到 `cargo` 时，执行 `cargo build --release` 并把 `target/release/arshy`、`target/release/arshyd` 复制到 `~/.local/bin`。
   - 否则从 GitHub Releases 下载 `arshy-v0.0.1-<target>.tar.gz` 并解压到 `~/.local/bin`。
3. 配置透明 shell 钩子（`arshy hook install`）。
4. 在 macOS 上对两个二进制做 ad-hoc 代码签名（见第 7 步）。
5. 打印完成信息。

预编译包支持的平台如下（映射来自 `install/install.sh` 与 `.github/workflows/release.yml`）：

| 系统 | 架构 | 下载的 target |
| --- | --- | --- |
| macOS | Apple Silicon（arm64） | `aarch64-apple-darwin` |
| macOS | Intel（x86_64） | `x86_64-apple-darwin` |
| Linux（含 WSL2） | x86_64 | `x86_64-unknown-linux-gnu` |
| Linux（含 WSL2） | ARM64 | `aarch64-unknown-linux-gnu` |

不支持的平台会打印 `Unsupported platform` 并退出。

### 安装脚本参数

```bash
# 安装后立即接入某个 agent（如 codex），并自动运行 doctor 验证
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh -s -- --agent codex

# 指定发布版本（默认 v0.0.1）
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh -s -- --version v0.0.1

# 只打印将要执行的操作，不改动任何文件
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh | sh -s -- --dry-run
```

`--agent` 支持的值与 `arshy setup` 一致：`codex`、`claude-code`、`cursor`、`vscode`、`antigravity`、`opencode`、`aider`、`workbuddy`。接入 agent 的详细流程见[把 Codex 接入 arshy](setup-agent.md)。

## 第 3 步：把 `~/.local/bin` 加入 PATH

脚本会提示你导出 PATH。把它写进你的 shell 配置，这样新终端也能用：

```bash
export PATH="$HOME/.local/bin:$PATH"
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc   # 或 ~/.bashrc
source ~/.zshrc                                            # 或 source ~/.bashrc
```

## 第 4 步：验证两个二进制

```bash
which arshy arshyd
arshy --version
```

预期输出类似：

```text
arshy v0.0.1
```

`which arshy arshyd` 应同时列出 `~/.local/bin/arshy` 和 `~/.local/bin/arshyd` 两条路径。

## 第 5 步：验证 daemon

`arshy status` 会连接 daemon 并返回 JSON：

```bash
arshy status
```

预期输出包含 `tasks_total`、`tasks_running`、`uptime_secs` 等字段。如果 daemon 未运行，`arshy run` 会在第一次执行命令时按需自动启动它——这是设计使然：**daemon 是按需自动启动的，不注册任何 OS 级（launchd/systemd）开机自启**。

## 第 6 步：跑第一条命令

```bash
arshy run "echo hello from arshy"
```

预期输出是一段 JSON，包含 `status: "completed"`、`exit_code: 0`、`raw_output: "hello from arshy"` 和 `task_id`。命令的输出格式与字段含义见[运行第一条命令](first-command.md)。

## 第 7 步：macOS ad-hoc 代码签名（注意点）

macOS 会静默杀掉未签名的 socket 绑定 daemon 进程。安装脚本和发布流水线都会对 macOS 二进制做 **ad-hoc 签名**（`codesign --force --deep -s -`），因此正常安装不需要额外操作。

但下面两种情况会让签名失效，需要你手动重新签名：

1. 你自己重新执行了 `cargo build --release` 并覆盖了 `~/.local/bin` 下的二进制。
2. 你从源码构建后手动复制二进制，而不是用安装脚本。

重新签名：

```bash
codesign --force --deep -s - ~/.local/bin/arshy
codesign --force --deep -s - ~/.local/bin/arshyd
```

验证签名（应看到 `Signature=adhoc`）：

```bash
codesign -dv ~/.local/bin/arshyd 2>&1 | grep Signature
```

如果跳过这一步，daemon 可能被 macOS 静默杀死，表现为 `DaemonUnreachable` 错误。遇到时先重新签名，再用 `arshy daemon restart` 重启。

## 第 8 步：运行 `arshy doctor` 完整体检

```bash
arshy doctor
```

doctor 逐项检查并给出修复提示：

- **1. Binaries**：`arshy` / `arshyd` 是否在 PATH。
- **2. Daemon**：daemon 是否在运行；不在运行会提示 `arshy daemon start`。
- **3. MCP server config**：Claude Code / Cursor 的 MCP 注册。
- **4. Permissions**：`arshy_exec` / `arshy_query` 等权限。
- **5. Filesystem access**：macOS TCC 目录访问限制。
- **5.5 Agent shell interception**：shell 钩子、当前工作区是否 opt-in、daemon 自动启动是否开启。
- **5.6 Agent integrations**：各 agent 的接入状态。
- **Summary**：最后一行形如 `3 passed, 0 warnings, 10 failed`。

**完成标准**：最后一行 `failed` 为 0。常见失败及修复：

| 检查失败 | 修复命令 |
| --- | --- |
| `arshy in PATH` / `arshyd in PATH` | 回到第 3 步配置 PATH |
| `daemon is running` | `arshy daemon start` |
| shell hook shims 未激活 | `arshy hook install`，然后重启终端 |
| 当前工作区未 opt-in | 在目标工作区运行 `arshy init` |
| agent 集成未激活 | `arshy setup <agent>`（见[把 Codex 接入 arshy](setup-agent.md)） |

## 其他安装方式

### 从源码构建

```bash
git clone https://github.com/iZoy/Arshy.git
cd Arshy
cargo build --release
```

产物在 `target/release/arshy` 和 `target/release/arshyd`。安装到 PATH 目录（二选一）：

```bash
# 方式 A：复制到 ~/.local/bin（与安装脚本一致）
mkdir -p ~/.local/bin
cp target/release/arshy target/release/arshyd ~/.local/bin/

# 方式 B：用 cargo 安装（安装到 ~/.cargo/bin）
cargo install --path .
```

然后执行 `arshy hook install` 配置 shell 钩子。macOS 用户记得执行第 7 步的重新签名。

### Homebrew

仓库里的 `Formula/arshy.rb` 是 Homebrew formula。当前版本从源码构建（`cargo install`），需要 Rust 工具链：

```bash
cd Arshy   # 克隆仓库后
brew install --build-from-source ./Formula/arshy.rb
```

formula 自带的测试会运行 `arshy --help` 并断言输出包含 `Arshy`。

## 故障排查

| 症状 | 处理 |
| --- | --- |
| 报错 `DaemonUnreachable` | 运行 `arshy daemon restart`；macOS 用户先检查代码签名（第 7 步） |
| `arshy: command not found` | 检查第 3 步的 PATH 配置并重启终端 |
| `Unsupported platform` | 你的平台没有预编译包；安装 Rust 工具链后重试（走源码构建） |
| WSL2 内 `command not found` | 确认你安装的是 WSL2 发行版内部，而不是 Windows 侧 |
| daemon 反复消失 | 检查是否有 stale spawn-lock（`/tmp/arshyd.spawn-lock`）；`arshy doctor` 会自动清除并提示重启 |

## 下一步

安装完成并体检通过后，进入[运行第一条命令](first-command.md)，体验短/长命令的结构化输出。
