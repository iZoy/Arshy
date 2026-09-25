# Manage the daemon

```bash
arshy daemon start
arshy status
arshy daemon restart
arshy daemon stop
```

The CLI or MCP proxy can auto-start `arshyd` when configured. The daemon owns
non-interactive `sh -c` execution over stdout/stderr pipes, parser sessions, event storage, and notifications; `arshy` is
the thin CLI/proxy front end.

The default lifecycle is on demand: `daemon.auto_start = true` starts the
daemon on the first connection, and `daemon.idle_timeout_secs = 300` exits it
after five minutes without task activity. Idle detection and Store flushing are
notification-driven, so no fixed per-second timer runs while the daemon waits.
Set the timeout to `0` only when a permanently resident daemon is intentional.

`store.integrity_check` is disabled by default because validating every stored
event makes cold-start time grow with history. Enable it for an explicit audit,
not for the normal on-demand path.

The daemon socket and store directory are configured under `[daemon]`. Use
`arshy doctor` for a read-only health check. No agent-specific service,
startup hook, or shell shim is installed.
