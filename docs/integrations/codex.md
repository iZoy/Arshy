# OpenAI Codex Integration

arshy integrates with Codex via `AGENTS.md` instructions.

Add to your project's `AGENTS.md`:

```markdown
## Shell Execution

Use arshy for all shell commands instead of raw shell:

- Build: `arshy run "cargo build"`
- Test: `arshy run "cargo test"`
- Lint: `arshy run "cargo clippy"`

For errors-only output: `arshy run "cargo build" --errors-only`
For JSON output: `arshy run "cargo build" --format json`
```

Codex will follow these instructions and use arshy for structured output.
