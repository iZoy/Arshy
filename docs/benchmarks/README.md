# Arshy agent diagnostic benchmark (v0)

Compare **native Codex shell** vs **forced `arshy_exec`** on ~20 small, intentionally broken projects.

Goal: measure whether structured diagnostics improve *primary-error identification* (success rate, fewer retries / tool rounds, optional token use) — not whether the agent can fix the bug.

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

`rust-type-mismatch` × `{shell, arshy}` both succeeded with path compliance:

- shell: ~151s, ~31.7k input / 193 output tokens, 1× `cargo check`
- arshy: ~208s, ~66.7k input / 297 output tokens, 1× `arshy_exec`

See `samples/rust-type-mismatch/`. Arshy input tokens are higher mainly because MCP tool schemas are injected; later turns may cache more.
