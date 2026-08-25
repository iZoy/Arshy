# Arshy

Arshy is a local structured command-execution and diagnostics layer for AI
agents, exposed through one thin, standard MCP server. It turns noisy command
output into typed events with locations, error codes, deduplication, and
optional source context.

This branch is the internal development candidate **v0.1.0-dev.1**. It is not a
public release, and its API carries no compatibility promise.

## Install

The installer only downloads and verifies the two binaries. It does not edit
agent configuration, shell startup files, hooks, or project files.

```sh
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh \
  | sh -s -- --version v0.1.0-dev.1
```

For a preview without changing the machine:

```sh
sh install/install.sh --dry-run
```

## Connect any MCP client

Register the same command in the MCP configuration of your client:

```json
{
  "mcpServers": {
    "arshy": {
      "command": "arshy",
      "args": ["mcp", "serve"]
    }
  }
}
```

`arshy mcp config` prints this registration as JSON. Arshy does not detect,
modify, or inject configuration for a named agent.

## CLI

```sh
arshy run "cargo test" --format json
arshy daemon start
arshy stats --format json
arshy analyze --format pretty
arshy benchmark
arshy doctor
```

The execution path returns raw-byte and structured-event counters. Efficiency
analytics are opt-in and appear only in `stats`, `analyze`, and benchmark
reports; no token estimate is emitted in MCP execution responses.

## Quality measurements

Analytics use the versioned `quality-v1` schema and expose independently
measured components: content convergence, noise filtering, diagnostic
completeness, and deduplication reduction. There is deliberately no aggregate
score, short/long-command percentage, or inferred repair-loop metric. These
values describe Arshy's event pipeline; they are not token-saving claims.

## Security boundary

Arshy is not an OS/container sandbox. Optional `security.allowed_cwds` is a
fail-closed working-directory guard; commands still run as the current user.
The command filter, path guard, Unix-socket peer check, and audit log are the
actual security controls.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo doc --no-deps --document-private-items
```

See [docs/getting-started.md](docs/getting-started.md),
[docs/reference/metrics.md](docs/reference/metrics.md), and
[docs/release-checklist.md](docs/release-checklist.md). Chinese readers can
start with [docs/zh/README.md](docs/zh/README.md).

## License

MIT. See [LICENSE](LICENSE).
