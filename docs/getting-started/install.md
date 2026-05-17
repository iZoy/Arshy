# 安装

## 方式一：Claude Code Plugin（推荐）

```bash
claude plugin install arshy@arshy-marketplace
```

安装后重启 Claude Code，运行 `/arshy-setup` 部署 daemon 二进制。

**无需修改 CLAUDE.md**。Arshy 的 MCP server 启动时自动宣告 "I am your shell"，agent 自然优先使用 arshy_exec。

## 方式二：从源码编译

```bash
git clone https://github.com/iZoy/arshy.git
cd arshy
cargo build --release
```

产物：
- `target/release/arshy` — CLI + MCP proxy
- `target/release/arshyd` — daemon

将两者放入 PATH：

```bash
cp target/release/arshy target/release/arshyd ~/.cargo/bin/
```

然后注册 MCP server：

```bash
arshy install
```

## 验证

```bash
arshy --version
arshy status
```

## 依赖

- Rust 1.75+
- SQLite（bundled，无需系统安装）
- macOS 或 Linux

## 目录结构

| 路径 | 用途 |
|------|------|
| `~/.config/arshy/config.toml` | 配置文件 |
| `~/.local/share/arshy/arshyd.sock` | Unix socket（权限 0600） |
| `~/.local/share/arshy/arshy.db` | SQLite 数据库（WAL 模式） |
| `~/.local/share/arshy/arshyd.pid` | PID 文件 |
| `~/.arshy/parsers/` | 用户自定义 parser（TOML / Rhai） |
| `~/.claude/plugins/arshy/` | 插件文件（skills、hooks） |
