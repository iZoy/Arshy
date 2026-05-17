# Contributing to Arshy

Arshy is an AI agent's native shell — we eat our own dog food. All contributions should be tested through arshy itself before submission.

## Development Setup

```bash
git clone https://github.com/iZoy/arshy.git
cd arshy
cargo build
cargo test
```

## Code Quality

- `cargo fmt` — format all code
- `cargo clippy -- -D warnings` — zero warnings
- `cargo test` — all tests must pass
- `cargo doc --no-deps` — no broken doc links

The CI workflow enforces all of these on every PR.

## Project Conventions

### Shell Execution (Dog Fooding)
Arshy's own development must use arshy for ALL shell commands. See `CLAUDE.md` for the full policy. This ensures every feature and fix is validated through real-world usage.

### Parser Contributions
When adding a new builtin parser:
1. Create `parsers/builtin/<tool>.toml` with `[meta]` and `[[pattern]]` sections
2. Create `parsers/builtin/tests/<tool>/` with `.txt` input and `.json` expected output
3. Run `ARSHY_BLESS=1 cargo test --bin arshyd` to auto-generate the expected JSON
4. Verify fixture tests pass with ≥95% field accuracy

### Commit Style
- `feat:` — new feature
- `fix:` — bug fix
- `docs:` — documentation
- `refactor:` — code change that neither fixes a bug nor adds a feature
- `test:` — adding tests

### Pattern Lifecycle
- Mark deprecated patterns with `deprecated = true` and `replaced_by = "new-pattern-name"`
- Never remove a pattern without a deprecation cycle
- Add `since_version` to new patterns

## Pull Requests

1. Ensure all CI checks pass
2. Update CHANGELOG.md under `[Unreleased]`
3. If adding a parser, include fixtures
4. If changing the TOML schema, bump `schema_version` in affected parser files
