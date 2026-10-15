# Arshy

Local CLI diagnostics for coding agents over MCP. Turns noisy build/test/lint output into structured evidence (severity, file, line, error code). **Portfolio / public preview** — not claiming mandatory agent infrastructure.

![Native cargo output compared with Arshy structured diagnostics](docs/assets/arshy-demo.gif)

The same failing `cargo check`, two ways. Natively, the agent reads the full compiler dump. Through Arshy it gets status, exit code, and a primary diagnostic (code, file, line, column); raw output stays available on request. Demo fixture: [agent-diagnostic-loop](docs/tutorials/agent-diagnostic-loop.md) (`scripts/record-demo.py`).

## Honest results (Codex, 2026-10-15)

On three **fix-until-pass** tasks (Rust multi-error, noisy pytest, Go multi-error), native shell and Arshy both fixed **3/3**. Arshy averaged fewer tool rounds (4.7 vs 5.7); total input tokens were similar (~416k vs ~426k); wall clock was slower for Arshy (~14 min vs ~11 min, with Codex reconnect noise). Diagnose-only smoke still shows a large MCP host/schema token tax. **Takeaway:** a working prototype with occasional round savings — not a proven cost or speed win, and not “must-install” infrastructure.

Full table: [docs/benchmarks/results-hard-fix-2026-10-15.md](docs/benchmarks/results-hard-fix-2026-10-15.md). Method: [docs/benchmarks/README.md](docs/benchmarks/README.md).

## What it is

- One MCP server (`arshy_exec`, `arshy_query`, `arshy_task`) plus a local daemon.
- ~38 built-in CLI parsers sharing the same diagnostic shape; duplicates collapsed.
- Evidence on demand: more events / raw capture only when the agent asks.
- Release path exercised end-to-end: musl static Linux builds, checksum-verified install script, GitHub Release [v0.1.0-alpha.1](https://github.com/iZoy/Arshy/releases/tag/v0.1.0-alpha.1).

v0.1.0-alpha.1 API may still change. macOS and Linux; Linux binaries are musl-static (Ubuntu 22.04 / Debian 12 friendly).

## Install

The installer downloads and verifies the arshy and arshyd binaries. It does not edit Agent configuration, shell profiles, hooks, or project files.

~~~bash
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/v0.1.0-alpha.1/install/install.sh \
  | bash -s -- --version v0.1.0-alpha.1
arshy daemon start
arshy doctor
~~~

Preview the download without changing the machine:

~~~bash
bash install/install.sh --version v0.1.0-alpha.1 --dry-run
~~~

## Connect an MCP client

From the project where the Agent will work, run:

~~~bash
arshy mcp config --format prompt
~~~

Give the complete output to the Agent. It contains the absolute executable path and instructs the Agent to use its client's native MCP registration mechanism. Codex and Claude Code are the release-gate clients for this preview; their real-session checks remain part of the checklist. Arshy has no client-specific adapter.

Successful registration does not load a new tool into an already-running session. Verify the MCP entry, then restart the client or open a new session. The new session should expose arshy_exec, arshy_query, and arshy_task. If an entry named arshy already exists with different values, inspect and resolve that conflict instead of overwriting it. Setup uses each client's native MCP configuration; exact steps vary by client.

## First task

~~~text
Use arshy_exec to run the project's test command. If it fails, inspect the
structured diagnostics, make the smallest justified fix, and run it again.
~~~

Execution results expose status and exit_code. Structured diagnostics use the MCP structuredContent field and remain visible as text for older clients. Use arshy_query for additional events and arshy_task with action raw for the captured output text.

See [the reproducible demo](docs/tutorials/agent-diagnostic-loop.md) and the [getting-started guide](docs/getting-started.md).

## Security and local data

Arshy runs commands with the current user's privileges. It is not an OS or container sandbox. security.allowed_cwds only checks the command's working directory.

Task metadata, events, raw output, version cache, and audit records are stored under the configured store directory (by default the platform's Arshy data directory). arshy prune removes task history according to the requested retention. Uninstalling the binaries does not delete stored data; review and remove the configured store directory separately if desired.

On Unix, the daemon restricts the store, `raw/`, and `events/` directories to the current user (`0700`) and store files to owner-only access (`0600`) when it opens the store. All processes running as the same user share this trust boundary.

arshy doctor --format json produces a local, minimal report containing only the version, OS/architecture, daemon and protocol check status, and parser count. Arshy does not upload diagnostics or telemetry.

## Project status and feedback

Shipped as a **portfolio / public preview**: end-to-end MCP + daemon + parsers + release artifacts. Early Codex benchmarks do not show a clear token or latency win over native shell; see the honest results above. No fixed token-saving claim.

- Report a bug, parser error, or real-task experience through [GitHub Issues](https://github.com/iZoy/Arshy/issues).
- Read [known issues](docs/known-issues.md), [security policy](SECURITY.md), and [contribution guidelines](CONTRIBUTING.md).
- Browse the [documentation index](docs/index.md) or the [中文指南](docs/zh/README.md).

## MCP Registry

- MCP Registry name: `mcp-name: io.github.iZoy/arshy`

The registry entry is described in [server.json](server.json).

## License

MIT. See [LICENSE](LICENSE).
