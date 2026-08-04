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
Arshy's own development must use arshy for ALL shell commands. See `AGENTS.md` for the full policy. This ensures every feature and fix is validated through real-world usage.

### Documentation Anti-Drift
Docs are **generated from source facts**, not prose memory. Before any PR that changes runtime behaviour, update the matching doc:

- **Storage changed** → `reference/config-schema.md` + `explanation/architecture.md`. The store is **JSONL**: `tasks.jsonl` / `events/<id>.jsonl` / `raw/<id>.txt` / `versions.json`. **Never** document SQLite, `arshy.db`, `wal_mode`, `db_path`, or `backend` — those are removed.
- **Parser changed** → `reference/parser-toml-format.md` + `explanation/parser-pipeline.md` (6-tier: JSON → Stateful → TOML → Crash → Heuristic → Raw; 37 builtin parsers).
- **IPC / MCP changed** → `reference/ipc-protocol.md` / `reference/mcp-protocol.md`.
- **Config keys / env vars changed** → `reference/config-schema.md`. Env vars are `ARSHY_<SECTION>_<KEY>`; the settable CLI keys live in `VALID_KEYS` in `src/cli/mod.rs`. Note `security.*` is **not** CLI-settable.
- The `TaskEvent.hint` / `EventHint` field is a **null wire-compat placeholder only** — do not document it as a feature; HintDb is removed.
- Run `cargo doc --no-deps` (enforced in CI) so doc cross-links stay valid.

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
