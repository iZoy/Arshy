# Metrics contract

Arshy does not publish a token-saving percentage. Tokenization varies by model,
protocol framing, and client rendering, so byte or word ratios are not a valid
universal claim.

## Execution counters

Run responses may include exact `raw_output_bytes`, structured event counts,
locations, codes, contexts, and deduplication counters. They describe that
command only and are not an efficiency claim.

## Quality components (`quality-v1`)

The explicit `stats`, `analyze`, and `benchmark` commands may include:

```json
{
  "schema_version": "quality-v1",
  "components": {
    "content_convergence_pct": 81.2,
    "noise_filter_pct": 34.0,
    "diagnostic_completeness_pct": 76.5,
    "dedup_reduction_pct": 9.1
  },
  "counters": {}
}
```

Components are reported independently. Missing denominators are `null`; no
weighted aggregate is computed. Command retry counts, short/long-command
percentages, and repair-loop inference are intentionally not part of the
public metrics contract because they depend on heuristics and workload
ordering.

These are engineering indicators, not token measurements. Publish the schema
version, workload, platform, sample size, and raw counters together.

## Reproducible quality workload

```sh
ARSHY=./target/release/arshy JSON=1 scripts/measure-savings.sh \
  > docs/evidence/quality-v1.json
```

The script reports per-ecosystem command outcomes and the component-only
quality report. It does not estimate tokens.
