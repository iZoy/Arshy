# Context Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enrich error events with surrounding source lines and correlate error files with recent git changes for faster diagnosis.

**Architecture:** Post-pipeline enrichment in executor. After events are parsed and stored, a ContextEnricher reads source files for error locations and populates EventContext. A GitCorrelator enhances project_context with changed file list and error correlation. Both reuse existing infrastructure (`context::extract_context`, `compute_project_context`).

**Tech Stack:** Rust, tokio::fs (async file reads), std::process::Command (git), HashMap cache

**Spec:** `docs/superpowers/specs/2026-06-10-context-enrichment-design.md`

---

## File Structure

| File | Action | Purpose |
|------|--------|---------|
| `src/daemon/context/mod.rs` | Modify | Add ContextEnricher struct, re-export |
| `src/daemon/context/git_correlator.rs` | Create | Git diff parsing + error correlation |
| `src/daemon/exec/mod.rs` | Modify | Wire enrichment into run_background |

Note: `src/daemon/context/mod.rs` already exists with `extract_context()` and `extract_context_async()` functions. We build on top of these.

---

### Task 1: ContextEnricher

**Files:**
- Modify: `src/daemon/context/mod.rs` (add ContextEnricher struct)
- Test: inline tests in `src/daemon/context/mod.rs`

- [ ] **Step 1: Write tests for ContextEnricher**

Add to `src/daemon/context/mod.rs` tests module:

```rust
#[test]
fn enricher_populates_context_for_errors() {
    use arshy_lib::ipc::{EventLocation, TaskEvent};

    let mut enricher = ContextEnricher::new(3);
    let mut events = vec![TaskEvent {
        seq: 0,
        event_type: "diagnostic".into(),
        severity: Some("error".into()),
        code: None,
        message: "test error".into(),
        location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
        context: None,
    }];

    enricher.enrich(&mut events, std::path::Path::new("."));
    assert!(events[0].context.is_some());
    assert!(!events[0].context.as_ref().unwrap().line.is_empty());
}

#[test]
fn enricher_skips_info_events() {
    use arshy_lib::ipc::{EventLocation, TaskEvent};

    let mut enricher = ContextEnricher::new(3);
    let mut events = vec![TaskEvent {
        seq: 0,
        event_type: "log".into(),
        severity: Some("info".into()),
        code: None,
        message: "info message".into(),
        location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
        context: None,
    }];

    enricher.enrich(&mut events, std::path::Path::new("."));
    assert!(events[0].context.is_none());
}

#[test]
fn enricher_skips_events_without_location() {
    use arshy_lib::ipc::TaskEvent;

    let mut enricher = ContextEnricher::new(3);
    let mut events = vec![TaskEvent {
        seq: 0,
        event_type: "diagnostic".into(),
        severity: Some("error".into()),
        code: None,
        message: "no location".into(),
        location: None,
        context: None,
    }];

    enricher.enrich(&mut events, std::path::Path::new("."));
    assert!(events[0].context.is_none());
}

#[test]
fn enricher_skips_existing_context() {
    use arshy_lib::ipc::{EventContext, EventLocation, TaskEvent};

    let mut enricher = ContextEnricher::new(3);
    let mut events = vec![TaskEvent {
        seq: 0,
        event_type: "diagnostic".into(),
        severity: Some("error".into()),
        code: None,
        message: "already has context".into(),
        location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
        context: Some(EventContext {
            before: vec![],
            line: "existing".into(),
            after: vec![],
        }),
    }];

    enricher.enrich(&mut events, std::path::Path::new("."));
    assert_eq!(events[0].context.as_ref().unwrap().line, "existing");
}

#[test]
fn enricher_handles_missing_file() {
    use arshy_lib::ipc::{EventLocation, TaskEvent};

    let mut enricher = ContextEnricher::new(3);
    let mut events = vec![TaskEvent {
        seq: 0,
        event_type: "diagnostic".into(),
        severity: Some("error".into()),
        code: None,
        message: "missing file".into(),
        location: Some(EventLocation { file: "/nonexistent/file.rs".into(), line: 1, column: None }),
        context: None,
    }];

    enricher.enrich(&mut events, std::path::Path::new("."));
    assert!(events[0].context.is_none());
}

#[test]
fn enricher_caches_file_reads() {
    use arshy_lib::ipc::{EventLocation, TaskEvent};

    let mut enricher = ContextEnricher::new(3);
    let mut events = vec![
        TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "error 1".into(),
            location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
            context: None,
        },
        TaskEvent {
            seq: 1,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "error 2".into(),
            location: Some(EventLocation { file: "Cargo.toml".into(), line: 2, column: None }),
            context: None,
        },
    ];

    enricher.enrich(&mut events, std::path::Path::new("."));
    // Both should be enriched (cache hit on second read)
    assert!(events[0].context.is_some());
    assert!(events[1].context.is_some());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib --bin arshyd context -- --nocapture 2>&1 | tail -20`
Expected: compilation error (ContextEnricher not defined)

- [ ] **Step 3: Implement ContextEnricher**

In `src/daemon/context/mod.rs`, add before the tests module:

```rust
use arshy_lib::ipc::{EventContext, EventLocation, TaskEvent};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Enriches error/warning events with surrounding source context.
/// Caches file reads to avoid re-reading the same file for multiple errors.
pub struct ContextEnricher {
    context_lines: usize,
    file_cache: HashMap<PathBuf, Vec<String>>,
}

impl ContextEnricher {
    pub fn new(context_lines: usize) -> Self {
        Self { context_lines, file_cache: HashMap::new() }
    }

    /// Enrich error/warning events that have a file:line location.
    /// Populates EventContext with surrounding source lines.
    /// Skips events that already have context, aren't error/warning,
    /// or have no location.
    pub fn enrich(&mut self, events: &mut [TaskEvent], cwd: &Path) {
        for event in events.iter_mut() {
            if event.context.is_some() {
                continue;
            }
            if !matches!(event.severity.as_deref(), Some("error") | Some("warning")) {
                continue;
            }
            let loc = match &event.location {
                Some(loc) => loc,
                None => continue,
            };
            if let Some(ctx) = self.read_context(cwd, loc) {
                event.context = Some(ctx);
            }
        }
    }

    fn read_context(&mut self, cwd: &Path, loc: &EventLocation) -> Option<EventContext> {
        let path = cwd.join(&loc.file);
        let lines = self
            .file_cache
            .entry(path.clone())
            .or_insert_with(|| {
                std::fs::read_to_string(&path)
                    .ok()
                    .map(|c| c.lines().map(String::from).collect())
                    .unwrap_or_default()
            })
            .clone();
        if lines.is_empty() {
            return None;
        }
        let idx = loc.line.checked_sub(1)? as usize;
        if idx >= lines.len() {
            return None;
        }
        let before = lines[idx.saturating_sub(self.context_lines)..idx]
            .iter()
            .cloned()
            .collect();
        let line = lines[idx].clone();
        let after = lines[idx + 1..(idx + 1 + self.context_lines).min(lines.len())]
            .iter()
            .cloned()
            .collect();
        Some(EventContext { before, line, after })
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib --bin arshyd context -- --nocapture 2>&1 | tail -20`
Expected: All tests PASS

- [ ] **Step 5: Run clippy + fmt**

Run: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/daemon/context/mod.rs
git commit -m "feat(context): add ContextEnricher with file cache

Enriches error/warning events with 3 lines of source context before/after
the error location. Caches file reads for efficiency."
```

---

### Task 2: Git Correlator

**Files:**
- Create: `src/daemon/context/git_correlator.rs`
- Modify: `src/daemon/context/mod.rs` (add module declaration)
- Modify: `src/daemon/exec/mod.rs:691-713` (enhance compute_project_context)

- [ ] **Step 1: Write tests for GitCorrelator**

Create `src/daemon/context/git_correlator.rs`:

```rust
//! Git diff correlation — identifies which error files were recently changed.

use arshy_lib::ipc::TaskEvent;
use std::path::Path;

/// Correlates error events with recently changed files from git diff.
pub struct GitCorrelation {
    changed_files: Vec<String>,
}

/// A correlation result for one error file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CorrelatedError {
    pub file: String,
    pub recently_changed: bool,
}

impl GitCorrelation {
    /// Detect recently changed files from `git diff --name-only HEAD~1`.
    /// Returns None if git is not available or no changes.
    pub fn detect(cwd: Option<&Path>) -> Option<Self> {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["diff", "--name-only", "HEAD~1"]);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<String> = stdout.lines().filter(|l| !l.is_empty()).map(String::from).collect();
        if files.is_empty() {
            return None;
        }
        Some(Self { changed_files: files })
    }

    /// Correlate error events with changed files.
    pub fn correlate(&self, events: &[TaskEvent]) -> Vec<CorrelatedError> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for event in events {
            if event.severity.as_deref() != Some("error") {
                continue;
            }
            let file = match &event.location {
                Some(loc) => &loc.file,
                None => continue,
            };
            if seen.contains(file) {
                continue;
            }
            seen.insert(file.clone());
            results.push(CorrelatedError {
                file: file.clone(),
                recently_changed: self.changed_files.iter().any(|f| f == file),
            });
        }
        results
    }

    /// Get the list of changed files (for project_context output).
    pub fn changed_files(&self) -> &[String] {
        &self.changed_files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::ipc::EventLocation;

    fn make_error(file: &str, line: u64) -> TaskEvent {
        TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "test".into(),
            location: Some(EventLocation { file: file.into(), line, column: None }),
            context: None,
        }
    }

    #[test]
    fn correlate_matches_changed_files() {
        let gc = GitCorrelation {
            changed_files: vec!["src/main.rs".into(), "Cargo.toml".into()],
        };
        let events = vec![
            make_error("src/main.rs", 42),
            make_error("src/lib.rs", 10),
        ];
        let results = gc.correlate(&events);
        assert_eq!(results.len(), 2);
        assert!(results[0].recently_changed);
        assert!(!results[1].recently_changed);
    }

    #[test]
    fn correlate_deduplicates_files() {
        let gc = GitCorrelation {
            changed_files: vec!["src/main.rs".into()],
        };
        let events = vec![
            make_error("src/main.rs", 10),
            make_error("src/main.rs", 20),
        ];
        let results = gc.correlate(&events);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn correlate_skips_non_errors() {
        let gc = GitCorrelation {
            changed_files: vec!["src/main.rs".into()],
        };
        let mut event = make_error("src/main.rs", 10);
        event.severity = Some("warning".into());
        let results = gc.correlate(&[event]);
        assert!(results.is_empty());
    }

    #[test]
    fn changed_files_returns_list() {
        let gc = GitCorrelation {
            changed_files: vec!["a.rs".into(), "b.rs".into()],
        };
        assert_eq!(gc.changed_files().len(), 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib --bin arshyd git_correlator -- --nocapture 2>&1 | tail -20`
Expected: compilation error (module not declared)

- [ ] **Step 3: Declare module in context/mod.rs**

In `src/daemon/context/mod.rs`, add at the top (after existing imports):

```rust
pub mod git_correlator;
```

Run: `cargo test --lib --bin arshyd git_correlator -- --nocapture 2>&1 | tail -20`
Expected: All 4 tests PASS

- [ ] **Step 4: Enhance compute_project_context**

In `src/daemon/exec/mod.rs`, replace the existing `compute_project_context` function (lines 691-713) with an enhanced version that uses GitCorrelation:

```rust
fn compute_enhanced_project_context(
    status: &TaskStatus,
    cwd: Option<&std::path::Path>,
    events: &[serde_json::Value],
) -> Option<serde_json::Value> {
    if *status != TaskStatus::Failed {
        return None;
    }

    let mut context = serde_json::json!({});

    // Git diff stat (existing)
    let mut cmd = std::process::Command::new("git");
    cmd.args(["diff", "--stat", "HEAD~1"]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let diff_stat = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !diff_stat.is_empty() {
                context["git_diff_stat"] = serde_json::json!(diff_stat);
            }
        }
    }

    // Git correlation (new)
    if let Some(gc) = super::context::git_correlator::GitCorrelation::detect(cwd) {
        context["changed_files"] = serde_json::json!(gc.changed_files());

        // Convert serde_json::Value events back to TaskEvent-like access for correlation
        let correlated: Vec<serde_json::Value> = events
            .iter()
            .filter(|e| e.get("severity").and_then(|v| v.as_str()) == Some("error"))
            .filter_map(|e| {
                let file = e.get("location")?.get("file")?.as_str()?;
                let recently_changed = gc.changed_files().iter().any(|f| f == file);
                Some(serde_json::json!({
                    "file": file,
                    "recently_changed": recently_changed,
                }))
            })
            .collect();

        if !correlated.is_empty() {
            context["correlated_errors"] = serde_json::json!(correlated);
        }
    }

    if context.as_object().map_or(true, |m| m.is_empty()) {
        return None;
    }
    Some(context)
}
```

- [ ] **Step 5: Update call site in run_background**

In `src/daemon/exec/mod.rs`, find where `compute_project_context(&info.status)` is called (around line 501) and replace with:

```rust
project_context: compute_enhanced_project_context(
    &info.status,
    t.cwd.as_deref(),
    &events_json.as_ref().unwrap_or(&vec![]),
),
```

- [ ] **Step 6: Run full test suite**

Run: `cargo test --lib --bin arshy --bin arshyd 2>&1 | tail -10`
Expected: All tests PASS

- [ ] **Step 7: Run clippy + fmt**

Run: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 8: Commit**

```bash
git add src/daemon/context/git_correlator.rs src/daemon/context/mod.rs src/daemon/exec/mod.rs
git commit -m "feat(context): add git correlation for error-to-change mapping

GitCorrelator detects recently changed files via git diff --name-only
and correlates them with error locations. Enhanced project_context now
includes changed_files list and correlated_errors for each failed task."
```

---

### Task 3: Integration — Wire enrichment into run_background

**Files:**
- Modify: `src/daemon/exec/mod.rs` (add enrichment step after event storage)

- [ ] **Step 1: Write integration test**

In `src/daemon/exec/mod.rs` tests, add:

```rust
#[test]
fn context_enricher_applied_to_events() {
    use arshy_lib::ipc::{EventLocation, TaskEvent};
    use super::super::context::ContextEnricher;

    let mut enricher = ContextEnricher::new(3);
    let mut events = vec![TaskEvent {
        seq: 0,
        event_type: "diagnostic".into(),
        severity: Some("error".into()),
        code: None,
        message: "error".into(),
        location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
        context: None,
    }];

    enricher.enrich(&mut events, std::path::Path::new("."));
    let ctx = events[0].context.as_ref().expect("should have context");
    assert!(!ctx.line.is_empty());
}
```

- [ ] **Step 2: Run test to verify it works**

Run: `cargo test --lib --bin arshyd context_enricher_applied -- --nocapture 2>&1 | tail -10`
Expected: PASS (ContextEnricher already works from Task 1)

- [ ] **Step 3: Wire enrichment into run_background**

In `src/daemon/exec/mod.rs`, in `run_background()`, after events are collected and stored, add enrichment before building the RunResult. Find the section where events are queried from the store for the sync completion path. After fetching events_json, add:

```rust
// Enrich error events with source context
let mut enricher = super::context::ContextEnricher::new(3);
if let Some(ref mut evts) = events_json {
    // Convert JSON events to TaskEvent for enrichment
    let mut task_events: Vec<arshy_lib::ipc::TaskEvent> = evts
        .iter()
        .filter_map(|e| serde_json::from_value(e.clone()).ok())
        .collect();
    enricher.enrich(&mut task_events, t.cwd.as_deref().unwrap_or(std::path::Path::new(".")));
    // Convert back to JSON with enriched context
    *evts = task_events
        .into_iter()
        .map(|e| serde_json::to_value(&e).unwrap_or_default())
        .collect();
}
```

- [ ] **Step 4: Run full test suite**

Run: `cargo test --lib --bin arshy --bin arshyd 2>&1 | tail -10`
Expected: All tests PASS

- [ ] **Step 5: Run clippy + fmt**

Run: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add src/daemon/exec/mod.rs
git commit -m "feat(exec): wire context enrichment into run_background

Error/warning events with file:line locations are now enriched with
3 lines of surrounding source context before being returned to the agent."
```

---

## Final Verification

- [ ] **Run full test suite:** `cargo test --lib --bin arshy --bin arshyd`
- [ ] **Format check:** `cargo fmt --all -- --check`
- [ ] **Clippy:** `cargo clippy --all-targets -- -D warnings`
- [ ] **Manual test:** Run a command that produces errors and verify events have context
