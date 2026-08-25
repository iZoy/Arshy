# Configuration reference

Configuration is TOML. CLI flags override environment variables, which
override the user file, which overrides defaults.

```toml
[daemon]
socket_path = "~/.local/share/arshy/arshyd.sock"
idle_timeout_secs = 300

[store]
store_dir = "~/.local/share/arshy"
# Full event-history validation is opt-in because its startup cost grows with history.
integrity_check = false

[security]
# Optional working-directory guard. This is not an OS sandbox.
allowed_cwds = ["~/projects"]
audit_log = "~/.local/share/arshy/audit.jsonl"
```

`security.allowed_cwds` is empty by default. When non-empty, every command
must use a `cwd` inside one of the expanded roots; invalid configuration or an
outside path fails closed. Commands still run with the invoking user's normal
permissions.

`daemon.auto_start` defaults to `true`: the CLI/MCP proxy starts `arshyd` on
the first request. The event-driven idle deadline then exits it after five
minutes without task activity; `0` disables this behavior.

Parser and store settings are documented by `arshy config --help`. The public
configuration surface contains no agent identifiers, hooks, shell shims, or
installation mutations.
