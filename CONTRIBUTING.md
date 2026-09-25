# Contributing to Arshy

Contributions from the community are welcome. Use the documented Rust toolchain and CI checks below; no Arshy installation is required to build or test the project.

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

## Before Opening a Pull Request

Use the matching Issue form for a general bug, parser error, or real-task
experience. Review every command, output sample, and doctor report before
making it public. Arshy does not need source code, environment variables,
credentials, usernames, or absolute local paths to triage most reports.

Maintainers prioritize duplicate execution, uncontrolled processes, and data
problems first; installation, MCP connection, and missing diagnostics second.
New parsers are ranked by observed use.

## Project Conventions

### Documentation
Keep user and contributor documentation aligned with source behavior. Before a PR that changes runtime behavior, update the matching page:

- **Storage changed** → `docs/reference/config.md` + `docs/explanation/architecture.md`. The store is **JSONL**: `tasks.jsonl` / `events/<id>.jsonl` / `raw/<id>.txt` / `versions.json`. **Never** document SQLite, `arshy.db`, `wal_mode`, `db_path`, or `backend` — those are removed.
- **Parser changed** → `docs/reference/parsers.md` + `docs/explanation/parser-pipeline.md` (JSON → Stateful → TOML → Crash → Heuristic → Raw).
- **IPC / MCP changed** → `docs/reference/ipc.md` / `docs/reference/mcp.md`.
- **Config keys / env vars changed** → `docs/reference/config.md`. Env vars are `ARSHY_<SECTION>_<KEY>`; the settable CLI keys live in `VALID_KEYS` in `src/cli/mod.rs`. Note `security.*` is **not** CLI-settable.
- The `TaskEvent.hint` / `EventHint` field is a **null wire-compat placeholder only** — do not document it as a feature; HintDb is removed.
- Run `cargo doc --no-deps` (enforced in CI) so doc cross-links stay valid.

### Parser Contributions
When adding a new builtin parser:
1. Create `parsers/builtin/<tool>.toml` with `[meta]` and `[[pattern]]` sections
2. Create `parsers/builtin/tests/<tool>/` with `.txt` input and `.json` expected output
3. Run `ARSHY_BLESS=1 cargo test --bin arshyd` to auto-generate the expected JSON
4. Verify fixture tests pass with ≥95% field accuracy
5. Keep the fixture to the smallest reviewed output that reproduces the issue

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
