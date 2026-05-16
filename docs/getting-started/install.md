# 安装

## 从源码编译

```bash
git clone https://github.com/anthropics/arshy.git
cd arshy
cargo build --release
```

产物：
- `target/release/arshy` — CLI + MCP proxy
- `target/release/arshyd` — daemon

将两者放入 PATH：

```bash
cp target/release/arshy target/release/arshyd ~/.local/bin/
```

## 验证

```bash
arshy --version
```

## 依赖

- Rust 1.75+
- SQLite（bundled，无需系统安装）
- macOS 或 Linux

## 目录结构

安装后 arshy 使用以下 XDG 目录：

| 路径 | 用途 |
|------|------|
| `~/.config/arshy/config.toml` | 配置文件 |
| `~/.local/share/arshy/arshyd.sock` | Unix socket |
| `~/.local/share/arshy/arshy.db` | SQLite 数据库 |
| `~/.local/share/arshy/arshyd.pid` | PID 文件 |
| `~/.arshy/parsers/` | 用户自定义 parser |
