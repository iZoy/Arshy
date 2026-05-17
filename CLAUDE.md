# Arshy Project Rules

## Shell Execution (DOG FOODING)

arshy is our own product. We route ALL shell commands through it — this is both validation and quality assurance. The MCP server self-describes as a shell replacement; no additional agent configuration is needed beyond installing the plugin.

### Why
Every command exercised through arshy is a real-world test. Using Bash to build a shell replacement is self-defeating.

### Fallback
If `arshy_exec` returns `DaemonUnreachable`, run `scripts/restart.sh`. Only if that fails, use Bash as last resort.

### Commit Policy
- Run `cargo test` through arshy_exec before every commit
- Never commit if arshy_exec is unavailable (fix the daemon first)
