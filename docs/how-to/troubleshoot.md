# Troubleshoot

## Check the installation

```sh
arshy doctor
arshy daemon status
```

Confirm both binaries are on `PATH`, the data directory is writable, and the
Unix socket exists. `arshy daemon restart` is safe when configuration changes.

## MCP client cannot start the server

Run `arshy mcp config` and copy the exact JSON entry into the client's MCP
configuration. The command must be `arshy` with args `mcp serve`; Arshy does
not edit client files for you.

## A command is rejected

Inspect the audit log and check command-filter rules. If `security.allowed_cwds`
is non-empty, the request's cwd must resolve inside one of the configured
roots. This is a directory guard only, not a full sandbox.

## Analytics are empty

Run several completed commands first, then call `arshy stats --format json`.
A quality report exposes component denominators directly; running tasks and
commands without raw or structured output simply yield null components; the
response reports exclusions explicitly.

## Collect a reproducible report

```sh
ARSHY=./target/release/arshy JSON=1 scripts/measure-savings.sh
```
