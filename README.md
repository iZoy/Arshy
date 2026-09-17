# Arshy

Arshy is a local command-execution and diagnostics layer for AI agents. It exposes one standard MCP server and turns noisy CLI output into structured, traceable evidence: severity, file locations, error codes, deduplication, and optional source context.

v0.1.0-alpha.1 is a public preview. Its API may change while real-world use is validated. Arshy supports macOS and Linux.

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

Successful registration does not load a new tool into an already-running session. Verify the MCP entry, then restart the client or open a new session. The new session should expose arshy_exec, arshy_query, and arshy_task. If an entry named arshy already exists with different values, inspect and resolve that conflict instead of overwriting it.

## First task

~~~text
Use arshy_exec to run the project's test command. If it fails, inspect the
structured diagnostics, make the smallest justified fix, and run it again.
~~~

Execution results expose status and exit_code. Structured diagnostics use the MCP structuredContent field and remain visible as text for older clients. Use arshy_query for additional events and arshy_task with action raw for the captured original output.

See [the reproducible demo](docs/tutorials/agent-diagnostic-loop.md) and the [getting-started guide](docs/getting-started.md).

## Security and local data

Arshy runs commands with the current user's privileges. It is not an OS or container sandbox. security.allowed_cwds only checks the command's working directory.

Task metadata, events, raw output, version cache, and audit records are stored under the configured store directory (by default the platform's Arshy data directory). arshy prune removes task history according to the requested retention. Uninstalling the binaries does not delete stored data; review and remove the configured store directory separately if desired.

arshy doctor --format json produces a local, minimal report containing only the version, OS/architecture, daemon and protocol check status, and parser count. Arshy does not upload diagnostics or telemetry.

## Project status and feedback

The preview tests whether Agents repeatedly benefit from structured CLI diagnostics. Arshy does not claim a fixed token-saving percentage. Measurements must cover complete tasks, including retries and follow-up calls.

- Report a bug, parser error, or real-task experience through [GitHub Issues](https://github.com/iZoy/Arshy/issues).
- Read [known issues](docs/known-issues.md), [security policy](SECURITY.md), and [contribution guidelines](CONTRIBUTING.md).
- See the [roadmap](docs/ROADMAP-STRATEGY.md) for the public-preview criteria.

## License

MIT. See [LICENSE](LICENSE).
