# Configure Arshy

Create a TOML file at the platform-specific config path shown by
`arshy config path` (or pass the path supported by your shell wrapper).

```toml
[daemon]
socket_path = "~/.local/share/arshy/arshyd.sock"
idle_timeout_secs = 300

[store]
store_dir = "~/.local/share/arshy"
integrity_check = false

[security]
allowed_cwds = ["~/projects"]
```

`allowed_cwds` is an optional directory guard, not a sandbox. Empty means
permissive; non-empty means fail-closed containment checks for command cwd.
Restart the daemon after changing daemon or security settings:

```bash
arshy daemon restart
arshy doctor
```

No configuration key names an AI agent. MCP clients register `arshy mcp serve`
in their own configuration.
