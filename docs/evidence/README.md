# Evidence policy

Historical snapshots that used the retired token/fallback fields were removed
before v0.1.0. New evidence must be generated with
`JSON=1 scripts/measure-savings.sh` and include the quality-v1 schema, corpus,
platform, sample size, counters, components, and exclusions.

The evidence_snapshot script records the same metadata under snapshot for the
accumulated local daemon store. Its sample size is the number of retained tasks
at capture time; it is a traceable operational snapshot, not a controlled
benchmark corpus.
