# Complete-task efficiency experiment

Run each case from the same project state with the same Agent client, model,
client version, Arshy version, and platform:

1. simple inspection;
2. successful build;
3. failing build;
4. multiple diagnostics;
5. unknown tool or unparsed output;
6. long-running task.

Compare native shell with reasonable filtering, the current preview, and a
candidate response-budget policy. Record:

| Field | Native shell | Preview | Candidate |
|---|---:|---:|---:|
| Task success | | | |
| Input tokens | | | |
| Output tokens | | | |
| Tool calls | | | |
| Follow-up queries | | | |
| Repeated executions | | | |
| Elapsed time | | | |

Use the tokenizer/accounting reported by the selected client or provider.
Record unavailable values as missing; do not convert bytes into tokens. Evaluate
task success before cost and time.
