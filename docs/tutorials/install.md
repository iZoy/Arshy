# Install the internal v0.1.0-dev.1 candidate

Supported release targets are macOS arm64/x86_64 and Linux x86_64/aarch64.
The installer downloads the tagged archive, verifies its SHA-256 checksum, and
places `arshy` and `arshyd` in `~/.local/bin` by default.

```sh
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh \
  | sh -s -- --version v0.1.0-dev.1
```

Options:

```text
--version <tag>       release tag (default v0.1.0-dev.1)
--install-dir <path>  destination (default ~/.local/bin)
--dry-run             print the resolved download without changing files
```

The script never compiles the caller's current directory and never modifies
agent configuration, shell startup files, hooks, or project files. After
installing, add `arshy mcp serve` to the MCP client's own configuration and
run `arshy doctor`.
