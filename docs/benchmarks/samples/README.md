# Smoke samples

Checked-in excerpt from NotBot run `results/smoke-002` (2026-10-15, Codex + Arshy 0.1.0-alpha.1).

Task: `rust-type-mismatch` (`cargo check` on the demo E0308 fixture).

| mode | success | path_compliance | tool_rounds | input_tokens | output_tokens | wall clock |
|------|---------|-----------------|-------------|--------------|---------------|------------|
| shell | yes | yes (1 shell) | 1 | ~31.7k | ~193 | ~151s |
| arshy | yes | yes (1× arshy_exec) | 1 | ~66.7k | ~297 | ~208s |

Both last messages reported `EXIT_CODE: 101`, `CODE: E0308`, `LOCATION: src/main.rs:2`.

Full `events.jsonl` stays under gitignored `results/` (large / noisy).
