# Getting started

## 1. Install

```bash
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/v0.1.0-alpha.1/install/install.sh \
  | bash -s -- --version v0.1.0-alpha.1
arshy daemon start
arshy doctor
```

The installer verifies the release checksum and only places `arshy` and
`arshyd` in the selected bin directory.

## 2. Register MCP

Run `arshy mcp config --format prompt` from the project root and send the
complete output to the Agent. For manual setup, use
`arshy mcp config --format json` and add the entry to the client's native
configuration.

Registration changes the client configuration; it does not inject tools into
the current session. Verify the entry, restart the client or open a new
session, then confirm that all three Arshy tools are listed. If an existing
`arshy` entry differs, inspect the conflict rather than overwriting it.

## 3. Run and inspect

```bash
arshy run "cargo test" --format json
arshy stats --format pretty
arshy analyze --format pretty
```

Execution output contains structured diagnostics. Analytics are explicit and
use the component-only quality-v2 report; no token estimate is implied.

For a shareable local check, run `arshy doctor --format json`. Review the
output before attaching it to an issue. Arshy never uploads the report.

## Uninstall and retained data

Remove the MCP entry through the client, then delete the two installed
binaries. This does not delete local task history. Use `arshy config get
store.store_dir` before uninstalling to find the configured data directory;
review it and remove it separately if desired.
