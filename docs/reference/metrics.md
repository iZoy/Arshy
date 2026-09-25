# Metrics contract

Arshy does not publish a token-saving percentage. Tokenization varies by model,
protocol framing, and client rendering, so byte or word ratios are not a valid
universal claim.

## Execution counters

Run responses may include `raw_output_bytes`, structured event counts,
locations, codes, and deduplication counters. `raw_output_bytes` counts
bytes read from stdout and stderr before UTF-8 replacement, line truncation, or
the total-output capture limit, including bytes drained after capture stops to
keep child pipes from blocking. On timeout it covers bytes consumed before the
deadline; it does not promise byte-for-byte replay from `raw_output`. These
counters describe that command only and are not an efficiency claim.

## Quality components (`quality-v2`)

The explicit `stats`, `analyze`, and `benchmark` commands may include:

```json
{
  "schema_version": "quality-v2",
  "components": {
    "content_convergence_pct": 81.2,
    "noise_filter_pct": 34.0,
    "diagnostic_completeness_pct": null,
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

```bash
ARSHY=./target/release/arshy JSON=1 scripts/measure-savings.sh \
  > target/quality-v2.json
```

The script reports per-ecosystem command outcomes and the component-only
quality report. It does not estimate tokens. Diagnostic completeness is
unavailable because source-context enrichment has been removed.
