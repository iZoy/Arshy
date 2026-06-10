# Level 4: Error Correlation + Context Enrichment

**Date:** 2026-06-10
**Status:** Draft
**Parent:** Competitive Feature Parity (Level 3 features)

## Background

arshy's core value is **structured semantic extraction** from command output — turning raw text into typed events with severity, file, line, and code. Level 4 builds on this by enriching error events with **source context** and **git correlation**, making failures self-diagnosable without the agent needing to read files or run git commands.

This is the difference between "there's an error at src/foo.rs:42" and "there's an error at src/foo.rs:42, here's the code, and this file was changed in your last commit."

## Design

### Component 1: ContextEnricher

**New module:** `src/daemon/context/enricher.rs`

For each error/warning event that has a `file:line` location, read the source file and populate the existing `EventContext` field (which is currently always `None`).

```rust
pub struct ContextEnricher {
    context_lines: usize,                          // default: 3
    file_cache: HashMap<PathBuf, Vec<String>>,     // avoid re-reading same file
}

impl ContextEnricher {
    /// Enrich events with source context. Only touches error/warning events
    /// that have a location and no existing context.
    pub fn enrich(&mut self, events: &mut [TaskEvent], cwd: &Path);

    /// Read N lines before and after the error line from the source file.
    /// Returns None if file doesn't exist or line is out of range.
    fn read_context(&mut self, cwd: &Path, loc: &EventLocation) -> Option<EventContext>;
}
```

**Key decisions:**
- Only enriches events with severity `error` or `warning` (not info/log)
- File cache avoids reading the same file multiple times per task
- Gracefully skips if file doesn't exist or line is out of range
- `context_lines = 3` (before and after the error line)
- Uses existing `EventContext` type from `arshy_lib::ipc` (`before: Vec<String>`, `line: String`, `after: Vec<String>`)

### Component 2: Git Correlator

**New module:** `src/daemon/context/git_correlator.rs`

When a command fails, parse `git diff --name-only HEAD~1` to get recently changed files, then cross-reference with error locations.

```rust
pub struct GitCorrelation {
    changed_files: Vec<String>,
}

impl GitCorrelation {
    /// Parse git diff output to extract changed file list.
    pub fn detect(cwd: &Path) -> Option<Self>;

    /// Correlate error events with changed files.
    pub fn correlate(&self, events: &[TaskEvent]) -> Vec<CorrelatedError>;
}

pub struct CorrelatedError {
    pub file: String,
    pub recently_changed: bool,
}
```

**Integration with existing `project_context`:**

Enhance the `compute_project_context()` function to return richer data:

```json
{
  "git_diff_stat": " src/foo.rs | 5 +++--\n 1 file changed, 3 insertions(+), 2 deletions(-)",
  "changed_files": ["src/foo.rs", "Cargo.toml"],
  "correlated_errors": [
    {"file": "src/foo.rs", "recently_changed": true},
    {"file": "src/bar.rs", "recently_changed": false}
  ]
}
```

### Component 3: Executor Integration

**Modified:** `src/daemon/exec/mod.rs` in `run_background()`

```
run_background() flow:
  1. Spawn command via PTY
  2. Parse lines → events (pipeline: JSON → Stateful → TOML → Crash → Heuristic → Raw)
  3. Deduplicate events (Deduplicator)
  4. Store events in SQLite
  5. [NEW] ContextEnricher: read source files, populate EventContext
  6. [ENHANCED] compute_project_context: add git correlation
  7. Build RunResult with enriched events + correlated project_context
```

Enrichment happens AFTER storage (DB has original events, MCP response has enriched events). This keeps the DB lightweight while giving the agent full context.

### Module Structure

```
src/daemon/context/
  mod.rs              — module declaration
  enricher.rs         — ContextEnricher + source file reading
  git_correlator.rs   — GitCorrelation + diff parsing
```

## Example: Before vs After

### Before (current)

```json
{
  "events": [
    {"type": "diagnostic", "severity": "error", "message": "error[E0308]: mismatched types",
     "location": {"file": "src/main.rs", "line": 42, "column": 10}},
    {"type": "diagnostic", "severity": "error", "message": "error[E0425]: cannot find value `x`",
     "location": {"file": "src/main.rs", "line": 58}}
  ],
  "project_context": {
    "git_diff_stat": " src/main.rs | 3 ++-\n 1 file changed"
  }
}
```

### After (enriched)

```json
{
  "events": [
    {"type": "diagnostic", "severity": "error", "message": "error[E0308]: mismatched types",
     "location": {"file": "src/main.rs", "line": 42, "column": 10},
     "context": {
       "before": ["fn process(input: &str) -> Result<i32> {", "    let value = input.parse();", "    match value {"],
       "line": "        Ok(v) => v + \"hello\",",
       "after": ["            Err(e) => return Err(e.into()),", "        }", "    }"]
     }},
    {"type": "diagnostic", "severity": "error", "message": "error[E0425]: cannot find value `x`",
     "location": {"file": "src/main.rs", "line": 58},
     "context": {
       "before": ["    let result = process(input);", "    println!(\"{}\", result);", ""],
       "line": "    x += 1;",
       "after": ["", "    Ok(())", "}"]
     }}
  ],
  "project_context": {
    "git_diff_stat": " src/main.rs | 3 ++-\n 1 file changed",
    "changed_files": ["src/main.rs"],
    "correlated_errors": [
      {"file": "src/main.rs", "recently_changed": true},
      {"file": "src/main.rs", "recently_changed": true}
    ]
  }
}
```

The agent now sees:
1. The actual code around each error (no need to `Read` the file)
2. Which errors are in recently changed files (likely caused by the user)

## Testing

- Unit tests for ContextEnricher: file exists, file missing, line out of range, cache hit
- Unit tests for GitCorrelation: changed files parsing, correlation logic
- Integration test: run a command that produces errors, verify events have context
- Edge cases: errors in dependency code (file doesn't exist), errors without file:line

## Files Summary

| File | Action | Purpose |
|------|--------|---------|
| `src/daemon/context/mod.rs` | Create | Module declaration |
| `src/daemon/context/enricher.rs` | Create | Source context enrichment |
| `src/daemon/context/git_correlator.rs` | Create | Git diff correlation |
| `src/daemon/exec/mod.rs` | Modify | Wire enrichment into run_background |
| `src/daemon/main.rs` | Modify | Declare context module |
