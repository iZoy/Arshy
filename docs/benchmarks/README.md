# Arshy agent diagnostic benchmark (v0)

Compare **native Codex shell** vs **forced `arshy_exec`** on diagnose-only fixtures plus harder **fix-until-pass** tasks.

Goals:
- **diagnose** (~20 tasks): primary-error identification (success, rounds, tokens)
- **fix / hard** (6 tasks): edit until `verify` exits 0; score success rate, tool rounds, total tokens including retries

## Layout

```
docs/benchmarks/
  README.md           ← this file
  tasks.json / tasks.yaml   ← task catalog + expected diagnostics
  prompts/            ← shell vs arshy diagnose-only prompts
  fixtures/           ← mini broken projects (copied to a temp workdir per run)
  samples/            ← checked-in smoke outputs
  results/            ← local runs (gitignored)
scripts/benchmark/
  run.py              ← Codex driver
  score.py            ← metrics from --json events + last message
```

## Prerequisites (NotBot / macOS)

- Codex CLI logged in (`codex login` if needed), with `mcp_servers.arshy` pointing at `~/.local/bin/arshy`
- `arshy doctor` healthy (daemon running)
- Toolchains for the tasks you enable: `cargo`, `python3`, `go`, `make`, `node`/`npm`
- Optional / network: `ruff`, and npm installs for `tsc` / `eslint` / `jest` fixtures
- Python 3 stdlib only (`tasks.json`); optional PyYAML if you edit `tasks.yaml`

## Modes

| mode | Codex config | Prompt rule |
|------|----------------|-------------|
| `shell` | `mcp_servers.arshy.enabled=false` | use only built-in shell |
| `arshy` | arshy enabled; other MCP servers disabled | use only `arshy_exec` / `arshy_query` / `arshy_task` |

Both use: `codex exec --ephemeral --approve-for-me --skip-git-repo-check --json` with **stdin closed** (`DEVNULL`) so Codex does not hang on “Reading additional input from stdin…”.

## Smoke (1 task × 2 modes)

From the repo root on NotBot:

```bash
cd ~/Documents/arshy
python3 scripts/benchmark/run.py --task rust-type-mismatch --modes shell,arshy
python3 scripts/benchmark/score.py docs/benchmarks/results/<timestamp>
```

Copy a trimmed sample into `docs/benchmarks/samples/` if you want it in git:

```bash
RUN=docs/benchmarks/results/<timestamp>
mkdir -p docs/benchmarks/samples/rust-type-mismatch
cp "$RUN/rust-type-mismatch__arshy/last_message.txt" docs/benchmarks/samples/rust-type-mismatch/arshy_last_message.txt
cp "$RUN/rust-type-mismatch__shell/last_message.txt" docs/benchmarks/samples/rust-type-mismatch/shell_last_message.txt
# optionally keep usage-only excerpts, not full 80k-token event dumps
```


## Hard fix-until-pass

Six noisy / multi-error tasks tagged `fix` + `hard`. The runner copies the fixture, lets Codex edit, then independently runs `verify`:

```bash
python3 scripts/benchmark/run.py --tag hard --modes shell,arshy
python3 scripts/benchmark/score.py docs/benchmarks/results/<timestamp>
```

| id | verify |
|----|--------|
| fix-rust-multi-errors | `cargo check` |
| fix-rust-tests | `cargo test` |
| fix-python-pytest-noisy | `python3 -m pytest -q` |
| fix-go-multi-errors | `go build -o /dev/null .` |
| fix-python-import-chain | `python3 -m pytest -q` |
| fix-make-rust-cascade | `make check` |

Success for these is **`meta.verify_pass`** (independent exit 0), not the model's self-reported STATUS.

## Full suite

Offline-friendly (skip network + optional ruff):

```bash
python3 scripts/benchmark/run.py --tag '!network' --tag '!optional'
python3 scripts/benchmark/score.py docs/benchmarks/results/<timestamp>
```

List what would run:

```bash
python3 scripts/benchmark/run.py --list --tag '!network' --tag '!optional'
python3 scripts/benchmark/run.py --list   # all, still skips missing binaries
```

Dry-run (workdirs + prompts only, no Codex / no API cost):

```bash
python3 scripts/benchmark/run.py --task rust-type-mismatch --dry-run
```

## Metrics

| metric | meaning |
|--------|---------|
| `success` | last message / tool output matches expected exit, code, message needles, location |
| `primary_hit` | message + location matched |
| `path_compliance` | arshy mode actually called arshy_*; shell mode did not |
| `tool_rounds` | mcp_tool_call + command_execution completions |
| `retries` | identical command repeated |
| `input_tokens` / `output_tokens` / `cached_input_tokens` | from `turn.completed.usage` when present |

## Cost / time (rough)

One diagnose-only smoke on `gpt-5.6-luna`-class models previously used on the order of **~80k input tokens** (mostly cached on later turns) and **~400 output tokens**, wall clock **~1–3 minutes** including reconnect noise.  
Full suite ≈ **20 tasks × 2 modes ≈ 40 runs**. Expect **tens of minutes to a couple of hours** and non-trivial API spend; start with `--task` / `--tag smoke` before the full grid.

## Notes

- Fixtures are copied out of the git tree per run so `cargo`/`git` do not see the Arshy monorepo.
- `git-overwrite-merge` runs `setup.sh` to create a dirty merge conflict.
- Network fixtures (`tsc`, `eslint`, `jest`) run `npm install` in setup — first run needs network.
- Do not push results with secrets; `docs/benchmarks/results/` is gitignored.

## Smoke result (NotBot, 2026-10-15)

`rust-type-mismatch` × `{shell, arshy}` both succeeded with path compliance.

| build | mode | wall | input tok | cached | output | notes |
|-------|------|------|-----------|--------|--------|-------|
| Sep-18 fat binary | shell | 151s | 31,695 | 19,968 | 193 | |
| Sep-18 fat binary | arshy | 208s | 66,704 | 50,176 | 297 | full events + git `Changed files` from wrong repo |
| compact MCP (this branch) | shell | 173s | 32,497 | 15,104 | 214 | |
| compact MCP (this branch) | arshy | 218s | 64,603 | 45,312 | 406 | text=`failed (exit 101)\nE0308 src/main.rs:2 mismatched types` |

Compact defaults cut ~2k arshy input tokens and removed fat `structuredContent`, but the **~32k gap vs shell remains** — mostly Codex MCP host/schema tax on a one-shot diagnose task, not raw compiler output. Hard fix-until-pass tasks are where rounds/success should be compared next.

See `samples/rust-type-mismatch/`.
