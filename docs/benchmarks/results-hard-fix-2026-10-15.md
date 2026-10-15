# Hard fix-until-pass results (2026-10-15)

**Scope:** three tasks × two modes (Codex `exec`, ephemeral, path-forced shell vs Arshy MCP). Success = independent `verify` exit 0 after the agent edits the fixture.

**Client:** Codex CLI on NotBot (macOS arm64), Arshy `0.1.0-alpha.1` with compact MCP defaults. Run id: `hard-fix-20261015-163250` (local under `docs/benchmarks/results/`, gitignored).

**Notes:** this sample does not show token savings or faster wall clock; success rates tied.

## Aggregate

| mode | success | avg tool rounds | avg retries | total input tokens | total output tokens | wall sum |
|------|---------|-----------------|-------------|--------------------|---------------------|----------|
| arshy | 3/3 | 4.67 | 1.00 | 416,109 | 4,679 | ~14.0 min |
| shell | 3/3 | 5.67 | 0.67 | 425,557 | 4,994 | ~10.9 min |

Wall times include Codex reconnect timeouts on several runs; treat latency as noisy.

## Per task

| task | mode | verify | rounds | retries | input tok | elapsed (s) |
|------|------|--------|--------|---------|-----------|-------------|
| fix-go-multi-errors | arshy | pass | 3 | 1 | 102,971 | 228 |
| fix-go-multi-errors | shell | pass | 5 | 0 | 167,861 | 233 |
| fix-python-pytest-noisy | arshy | pass | 7 | 1 | 175,901 | 314 |
| fix-python-pytest-noisy | shell | pass | 9 | 2 | 190,242 | 239 |
| fix-rust-multi-errors | arshy | pass | 4 | 1 | 137,237 | 300 |
| fix-rust-multi-errors | shell | pass | 3 | 0 | 67,454 | 180 |

Go and Python: fewer rounds for Arshy. Rust: shell used fewer rounds and fewer tokens. Net: **tie on success, slight round edge for Arshy, no token/speed advantage.**

## Reproduce

```bash
python3 scripts/benchmark/run.py \
  --task fix-rust-multi-errors,fix-python-pytest-noisy,fix-go-multi-errors \
  --modes shell,arshy
python3 scripts/benchmark/score.py docs/benchmarks/results/<timestamp>
```

See [README.md](README.md) for harness details.
