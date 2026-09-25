# Arshy documentation

This index is for people installing, using, or contributing to Arshy. Start with
the setup guide, then use the task guides and reference pages as needed. The
[中文指南](zh/README.md) covers installation, MCP setup, data, and security in
Chinese.

## Start here

- [Getting started](getting-started.md) — install, register an MCP client, run a command, and understand retained data.
- [Install tutorial](tutorials/install.md) — verify the binary, check the daemon, upgrade, and uninstall.
- [First command tutorial](tutorials/first-command.md) — run short and structured commands and inspect task results.
- [Connect an Agent](tutorials/setup-agent.md) — configure a generic MCP client.
- [FAQ](faq.md) — common questions about compatibility, metrics, security, and removal.

## User guides

- [Configure Arshy](how-to/configure.md)
- [Manage the daemon](how-to/manage-daemon.md)
- [Integrate an MCP client](how-to/integrate-agent.md)
- [Security and the directory guard](how-to/security.md)
- [Troubleshoot](how-to/troubleshoot.md)

## Contributor guides

- [Contributing](../CONTRIBUTING.md) — development setup, pull requests, and project conventions.
- [Create a parser](how-to/create-parser.md) — add or customize a TOML parser and fixtures.
- [Run and extend tests](how-to/run-tests.md) — unit, integration, parser fixture, and dogfood checks.
- [Testing model](explanation/testing.md) — what each test layer verifies and where its limits are.

## Reference

- [CLI](reference/cli.md)
- [Configuration](reference/config.md)
- [MCP protocol](reference/mcp.md)
- [Daemon IPC](reference/ipc.md) — for integrations and contributors working on the protocol.
- [Parsers](reference/parsers.md)
- [Reference codes](reference/reference-codes.md)
- [Metrics contract](reference/metrics.md)

## Concepts and architecture

- [Architecture](explanation/architecture.md)
- [Parser pipeline](explanation/parser-pipeline.md)
- [Integration model](explanation/integration-model.md)
- [Security model](explanation/security-model.md)
- [Design principles](explanation/design-principles.md)

## Project information

- [Known issues](known-issues.md)
- [Security policy](../SECURITY.md)
- [Changelog](../CHANGELOG.md)
- [Architecture decisions](decisions/README.md) — accepted technical decisions and their trade-offs.

Pages under `reference/` and `explanation/` describe the current implementation;
when behavior changes, the source code and tests are authoritative.
