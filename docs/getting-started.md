# Getting started

## 1. Install

```sh
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/main/install/install.sh \
  | sh -s -- --version v0.1.0-dev.1
arshy doctor
```

The installer verifies the release checksum and only places `arshy` and
`arshyd` in the selected bin directory.

## 2. Register MCP

Run `arshy mcp config`, then add the JSON entry to any MCP-compatible client.
The command is intentionally identical across ecosystems.

## 3. Run and inspect

```sh
arshy run "cargo test" --format json
arshy stats --format pretty
arshy analyze --format pretty
```

Execution output contains structured diagnostics. Analytics are explicit and
use the component-only quality-v1 report; no token estimate is implied.
