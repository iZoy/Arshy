# GenericPairMerger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge diagnostic + location event pairs into single rich events so agents get complete error information in one event instead of two.

**Architecture:** A new `GenericPairMerger` post-processor runs after `RustcContextMerger` in the parser pipeline. It buffers the previous event and detects diagnostic+location pairs in both orderings (cargo: diagnostic→location, python: location→diagnostic), merging them into a single diagnostic event with location attached.

**Tech Stack:** Rust, existing parser pipeline (`src/daemon/parser/`), JSONL fixture tests (`parsers/builtin/tests/`)

## Global Constraints

- `rustfmt.toml`: max_width=100, 4 spaces, Unix newlines
- `clippy.toml`: allow-unwrap-in-tests=true
- CI: `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, unit tests, doc build
- Commit style: `feat:`, `fix:`, `refactor:`, `test:`
- Fixture accuracy: ≥95% field match rate per parser
- ARSHY_BLESS=1 to regenerate fixture expected JSON

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `src/daemon/parser/pair_merger.rs` | **Create** | GenericPairMerger struct + merge logic + unit tests |
| `src/daemon/parser/mod.rs` | **Modify** | Add `pub mod pair_merger;` |
| `src/daemon/exec/mod.rs` | **Modify** | Integrate pair merger into event pipeline |
| `src/daemon/store/mod.rs` | **Modify** | Add `pairs_merged: u64` to `TaskMetrics` |
| `parsers/builtin/tests/cargo/*.json` | **Regenerate** | Merged diagnostic+location events |
| `parsers/builtin/tests/python/*.json` | **Regenerate** | Merged location+diagnostic events |
| `parsers/builtin/tests/clippy/*.json` | **Regenerate** | If has location events |
| `parsers/builtin/tests/docker/*.json` | **Regenerate** | If has location events |
| `parsers/builtin/tests/terraform/*.json` | **Regenerate** | If has location events |
| `parsers/builtin/tests/oxlint/*.json` | **Regenerate** | If has location events |
| `parsers/builtin/tests/helm/*.json` | **Regenerate** | If has location events |
| `parsers/builtin/tests/git/*.json` | **Regenerate** | If has location events |

---

### Task 1: Create GenericPairMerger module with unit tests

**Files:**
- Create: `src/daemon/parser/pair_merger.rs`
- Modify: `src/daemon/parser/mod.rs` (add module declaration)

**Interfaces:**
- `GenericPairMerger` struct with `feed(event) -> Option<TaskEvent>` and `finish() -> Option<TaskEvent>` — for streaming per-event usage in exec pipeline
- `merge_diagnostic_location_pairs(Vec<TaskEvent>) -> (Vec<TaskEvent>, u64)` — batch function for fixture tests
- Helper: `fn is_diagnostic(e: &TaskEvent) -> bool` — matches `"diagnostic" | "crash"`
- Helper: `fn is_location(e: &TaskEvent) -> bool` — matches `"location"`

- [ ] **Step 1: Add module declaration to `src/daemon/parser/mod.rs`**

Find the existing `pub mod` declarations near the top of the file (around line 1-10). Add:

```rust
pub mod pair_merger;
```

- [ ] **Step 2: Create `src/daemon/parser/pair_merger.rs`**

```rust
//! Post-processor that merges diagnostic + location event pairs.
//!
//! Tools like cargo emit errors as two consecutive events:
//!   diagnostic { code, message } → location { file, line }
//! Tools like python emit them in reverse:
//!   location { file, line } → diagnostic { message }
//!
//! This merger detects both orderings and combines them into a single
//! diagnostic event with location attached, reducing the number of events
//! agents need to correlate.

use crate::ipc::TaskEvent;

/// Returns true if the event is a diagnostic or crash type.
fn is_diagnostic(e: &TaskEvent) -> bool {
    matches!(e.event_type.as_str(), "diagnostic" | "crash")
}

/// Returns true if the event is a location type.
fn is_location(e: &TaskEvent) -> bool {
    e.event_type == "location"
}

/// Streaming pair merger — use `feed()` per event, `finish()` at end.
///
/// Follows the same pattern as `RustcContextMerger`:
/// - Buffers one pending event (diagnostic or location)
/// - On next event, decides whether to merge or flush
/// - Returns `None` if the event was absorbed, `Some(event)` if emitted
pub struct GenericPairMerger {
    pending: Option<TaskEvent>,
    merged_count: u64,
}

impl GenericPairMerger {
    pub fn new() -> Self {
        Self { pending: None, merged_count: 0 }
    }

    /// Feed one event through the merger. Returns:
    /// - `None` if the event was absorbed into the pending buffer
    /// - `Some(event)` if an event should be emitted (may be the merged result)
    pub fn feed(&mut self, event: TaskEvent) -> Option<TaskEvent> {
        if is_location(&event) {
            if let Some(pending) = self.pending.take() {
                if is_diagnostic(&pending) {
                    // Forward merge: diagnostic → location
                    let mut merged = pending;
                    if merged.location.is_none() {
                        merged.location = event.location.clone();
                    }
                    self.merged_count += 1;
                    return Some(merged);
                } else {
                    // Pending is not a diagnostic: flush pending, buffer location
                    self.pending = Some(event);
                    return Some(pending);
                }
            } else {
                // No pending: buffer location for potential backward merge
                self.pending = Some(event);
                return None;
            }
        }

        if is_diagnostic(&event) {
            if let Some(pending) = self.pending.take() {
                if is_location(&pending) {
                    // Backward merge: location → diagnostic
                    let mut merged = event;
                    if merged.location.is_none() {
                        merged.location = pending.location.clone();
                    }
                    self.merged_count += 1;
                    return Some(merged);
                } else {
                    // Pending is not a location: flush pending, emit current
                    let flushed = Some(pending);
                    self.pending = Some(event);
                    return flushed;
                }
            } else {
                // No pending: buffer diagnostic for potential forward merge
                self.pending = Some(event);
                return None;
            }
        }

        // Non-pairable event: flush pending, emit current
        let flushed = self.pending.take();
        if flushed.is_some() {
            // Return flushed event first; current event will come on next call.
            // But we can't return two events from one call. So push current
            // into pending and return flushed. Next call will handle current.
            // Actually, simpler: just return flushed and let caller re-feed current.
            // But the caller iterates sequentially, so we need to handle this.
            // Solution: store current in pending, return flushed.
            // The caller will call feed() again with the next event, and pending
            // will be flushed then.
            //
            // WAIT — this doesn't work because we've already consumed `event`.
            // We need a different approach for the non-pairable case.
        }
        // Simpler: flush pending and emit current directly
        if let Some(p) = self.pending.take() {
            // We have a pending event AND a non-pairable current event.
            // We need to return both, but can only return one.
            // Store current in pending temporarily, return the old pending.
            // The caller must call finish() or feed() again.
            // Actually, this is the same problem as RustcContextMerger.
            // Let's look at how it handles this...
            //
            // RustcContextMerger only buffers diagnostic/log events.
            // It never buffers non-diagnostic events. If current is not
            // a context line, it flushes pending and returns the flushed
            // event. The CURRENT event is returned on the NEXT feed() call
            // by storing it in pending.
            //
            // We can't do that here because we've already taken `event`.
            // The solution: return flushed, and the caller will feed() the
            // same event again? No, the caller iterates.
            //
            // Correct solution: store `event` in pending, return old pending.
            self.pending = Some(event);
            return Some(p);
        }
        Some(event)
    }

    /// Flush any remaining pending event. Call at end of stream.
    pub fn finish(&mut self) -> Option<TaskEvent> {
        self.pending.take()
    }

    /// Returns the number of pairs merged so far.
    pub fn merged_count(&self) -> u64 {
        self.merged_count
    }
}

/// Batch merge function — for use in fixture tests where events are
/// collected into a Vec before processing.
///
/// Returns (merged_events, pairs_merged_count).
pub fn merge_diagnostic_location_pairs(events: Vec<TaskEvent>) -> (Vec<TaskEvent>, u64) {
    let mut merger = GenericPairMerger::new();
    let mut result = Vec::with_capacity(events.len());
    for event in events {
        if let Some(emitted) = merger.feed(event) {
            result.push(emitted);
        }
    }
    if let Some(final_event) = merger.finish() {
        result.push(final_event);
    }
    (result, merger.merged_count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::EventLocation;

    fn diag(code: Option<&str>, msg: &str) -> TaskEvent {
        TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: code.map(|s| s.into()),
            message: msg.into(),
            location: None,
            context: None,
            hint: None,
        }
    }

    fn loc(file: &str, line: u64) -> TaskEvent {
        TaskEvent {
            seq: 0,
            event_type: "location".into(),
            severity: Some("info".into()),
            code: None,
            message: format!("  --> {}:{}", file, line),
            location: Some(EventLocation {
                file: file.into(),
                line,
                column: None,
            }),
            context: None,
            hint: None,
        }
    }

    fn summary(msg: &str) -> TaskEvent {
        TaskEvent {
            seq: 0,
            event_type: "summary".into(),
            severity: Some("info".into()),
            code: None,
            message: msg.into(),
            location: None,
            context: None,
            hint: None,
        }
    }

    #[test]
    fn forward_merge_diagnostic_then_location() {
        let events = vec![diag(Some("E0308"), "mismatched types"), loc("src/main.rs", 42)];
        let (merged, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(merged.len(), 1);
        assert_eq!(count, 1);
        assert_eq!(merged[0].event_type, "diagnostic");
        assert_eq!(merged[0].code.as_deref(), Some("E0308"));
        assert!(merged[0].location.is_some());
        assert_eq!(merged[0].location.as_ref().unwrap().file, "src/main.rs");
        assert_eq!(merged[0].location.as_ref().unwrap().line, 42);
    }

    #[test]
    fn backward_merge_location_then_diagnostic() {
        let events = vec![loc("foo.py", 15), diag(None, "SyntaxError: invalid syntax")];
        let (merged, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(merged.len(), 1);
        assert_eq!(count, 1);
        assert_eq!(merged[0].event_type, "diagnostic");
        assert_eq!(merged[0].message, "SyntaxError: invalid syntax");
        assert!(merged[0].location.is_some());
        assert_eq!(merged[0].location.as_ref().unwrap().file, "foo.py");
        assert_eq!(merged[0].location.as_ref().unwrap().line, 15);
    }

    #[test]
    fn consecutive_locations_flush_first() {
        let events = vec![
            diag(Some("E0308"), "mismatched types"),
            loc("src/a.rs", 10),
            loc("src/b.rs", 20),
        ];
        let (merged, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(merged.len(), 2);
        assert_eq!(count, 1);
        assert_eq!(merged[0].location.as_ref().unwrap().file, "src/a.rs");
        assert_eq!(merged[1].location.as_ref().unwrap().file, "src/b.rs");
    }

    #[test]
    fn consecutive_diagnostics_flush_first() {
        let events = vec![
            diag(Some("E0308"), "mismatched types"),
            diag(Some("E0412"), "cannot find type"),
            loc("src/main.rs", 42),
        ];
        let (merged, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(merged.len(), 2);
        assert_eq!(count, 1);
        assert!(merged[0].location.is_none());
        assert!(merged[1].location.is_some());
    }

    #[test]
    fn no_pair_unchanged() {
        let events = vec![diag(Some("E0308"), "mismatched types"), summary("build done")];
        let (merged, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(merged.len(), 2);
        assert_eq!(count, 0);
    }

    #[test]
    fn empty_input() {
        let events: Vec<TaskEvent> = vec![];
        let (merged, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(merged.len(), 0);
        assert_eq!(count, 0);
    }

    #[test]
    fn does_not_overwrite_existing_location() {
        let mut diag_with_loc = diag(Some("E0308"), "mismatched types");
        diag_with_loc.location = Some(EventLocation {
            file: "existing.rs".into(),
            line: 1,
            column: None,
        });
        let events = vec![diag_with_loc, loc("new.rs", 42)];
        let (merged, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(merged.len(), 1);
        assert_eq!(count, 1);
        assert_eq!(merged[0].location.as_ref().unwrap().file, "existing.rs");
    }

    #[test]
    fn python_multi_frame_traceback() {
        let events = vec![
            diag(None, "Traceback (most recent call last):"),
            loc("foo.py", 15),
            loc("baz.py", 20),
            diag(None, "SomeError: message"),
        ];
        let (merged, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(merged.len(), 2);
        assert_eq!(count, 2);
        assert_eq!(merged[0].location.as_ref().unwrap().file, "foo.py");
        assert_eq!(merged[0].location.as_ref().unwrap().line, 15);
        assert_eq!(merged[1].location.as_ref().unwrap().file, "baz.py");
        assert_eq!(merged[1].location.as_ref().unwrap().line, 20);
    }

    #[test]
    fn feed_streaming_forward_merge() {
        let mut merger = GenericPairMerger::new();
        // diagnostic buffered (returns None)
        assert!(merger.feed(diag(Some("E0308"), "err")).is_none());
        // location merges with pending diagnostic
        let merged = merger.feed(loc("src/main.rs", 42)).unwrap();
        assert_eq!(merged.event_type, "diagnostic");
        assert!(merged.location.is_some());
        assert_eq!(merged.merged_count(), 1);
        // finish: nothing left
        assert!(merger.finish().is_none());
    }

    #[test]
    fn feed_streaming_backward_merge() {
        let mut merger = GenericPairMerger::new();
        // location buffered (returns None)
        assert!(merger.feed(loc("foo.py", 15)).is_none());
        // diagnostic merges with pending location
        let merged = merger.feed(diag(None, "SyntaxError")).unwrap();
        assert_eq!(merged.event_type, "diagnostic");
        assert!(merged.location.is_some());
        assert_eq!(merged.location.as_ref().unwrap().file, "foo.py");
        assert_eq!(merged.merged_count(), 1);
    }

    #[test]
    fn feed_streaming_flush_on_non_pairable() {
        let mut merger = GenericPairMerger::new();
        // diagnostic buffered
        assert!(merger.feed(diag(Some("E0308"), "err")).is_none());
        // summary forces flush of pending diagnostic
        let flushed = merger.feed(summary("done")).unwrap();
        assert_eq!(flushed.event_type, "diagnostic");
        // summary is now in pending, finish returns it
        let last = merger.finish().unwrap();
        assert_eq!(last.event_type, "summary");
    }
}
```

**Note on the `feed()` implementation:** The `feed()` method handles non-pairable events by storing the current event in `pending` and returning the old pending event. This means a non-pairable event may be returned on the *next* `feed()` call rather than immediately. The `finish()` method flushes any remaining pending event at end of stream. This is the same pattern used by `RustcContextMerger`.

- [ ] **Step 3: Run unit tests to verify they pass**

Run: `cargo test --lib pair_merger -- --nocapture 2>&1`
Expected: All 11 tests pass (8 batch + 3 streaming).

- [ ] **Step 4: Run clippy and format check**

Run: `cargo clippy --all-targets -- -D warnings 2>&1 && cargo fmt --all -- --check 2>&1`
Expected: Zero warnings, format OK. Note: the verbose `if let` chains in `feed()` may trigger clippy lints — refactor to match the style of `RustcContextMerger::feed()` in `src/daemon/parser/mod.rs:366-387`.

- [ ] **Step 5: Commit**

```bash
git add src/daemon/parser/pair_merger.rs src/daemon/parser/mod.rs
git commit -m "feat: add GenericPairMerger with feed() streaming and batch merge for diagnostic+location pairs"
```

---

### Task 2: Integrate pair merger into exec pipeline

**Files:**
- Modify: `src/daemon/exec/mod.rs:1048-1049` (instantiation)
- Modify: `src/daemon/exec/mod.rs:1107-1167` (per-event pipeline)
- Modify: `src/daemon/exec/mod.rs:1170-1203` (stream-end flush)

**Interfaces:**
- Consumes: `GenericPairMerger::new()`, `.feed(event) -> Option<TaskEvent>`, `.finish() -> Option<TaskEvent>`, `.merged_count() -> u64`
- Produces: merged events stored in JSONL in real-time, `pairs_merged` count for metrics
- Preserves: real-time EventBus streaming (events published as they arrive, not batched)

- [ ] **Step 1: Add import at the top of `src/daemon/exec/mod.rs`**

Find the existing `use` statements for parser modules. Add:

```rust
use crate::daemon::parser::pair_merger::GenericPairMerger;
```

- [ ] **Step 2: Instantiate GenericPairMerger alongside Deduplicator and RustcContextMerger**

In `run_background()`, near line 1048-1049 where `dedup` and `ctx_merger` are created, add:

```rust
let mut pair_merger = GenericPairMerger::new();
```

- [ ] **Step 3: Chain pair_merger.feed() after ctx_merger.feed() in the per-event loop**

The current per-event loop (lines 1107-1167) looks approximately like:

```rust
for event in events {
    if let Some(deduped) = dedup.feed(event) {
        if let Some(mut event) = ctx_merger.feed(deduped) {
            seq += 1;
            event.seq = seq;
            // ... stderr promotion, counting, context extraction, store, publish
        }
    }
}
```

Add a third chain link after `ctx_merger.feed()`:

```rust
for event in events {
    if let Some(deduped) = dedup.feed(event) {
        if let Some(ctx_merged) = ctx_merger.feed(deduped) {
            if let Some(mut event) = pair_merger.feed(ctx_merged) {
                seq += 1;
                event.seq = seq;
                // ... stderr promotion, counting, context extraction, store, publish
                // (all existing logic preserved exactly as-is)
            }
        }
    }
}
```

**Key:** All existing logic (stderr severity promotion, error/warning counting, async context extraction, `store.insert_event`, `event_bus.publish`) stays exactly as-is, just nested one level deeper.

- [ ] **Step 4: Add pair_merger.finish() to the stream-end flush**

The existing stream-end flush (lines 1170-1203) calls `dedup.finish()` and `ctx_merger.finish()`. Add `pair_merger.finish()` after them:

```rust
// Existing:
if let Some(final_event) = dedup.finish() {
    if let Some(merged_event) = ctx_merger.feed(final_event) {
        // ... store this event
    }
}
if let Some(final_event) = ctx_merger.finish() {
    // ... store this event
}

// New: flush pair_merger
if let Some(final_event) = pair_merger.finish() {
    seq += 1;
    let mut event = final_event;
    event.seq = seq;
    // ... stderr promotion, counting, context extraction, store, publish
    // (same logic as the per-event loop body)
}
```

- [ ] **Step 5: Write `pairs_merged` count to metrics**

After the stream-end flush, before the function returns, update the task metrics:

```rust
let pairs_merged = pair_merger.merged_count();
if pairs_merged > 0 {
    // Update task metrics — follow the same pattern as other metric writes
    // in this function (look for `record.metrics.` patterns)
}
```

Check how `dedup_collapsed` is written to the task record — follow the same pattern for `pairs_merged`.

- [ ] **Step 6: Run cargo build**

Run: `cargo build 2>&1`
Expected: Compiles without errors.

- [ ] **Step 7: Run clippy and format check**

Run: `cargo clippy --all-targets -- -D warnings 2>&1 && cargo fmt --all -- --check 2>&1`
Expected: Zero warnings.

- [ ] **Step 8: Commit**

```bash
git add src/daemon/exec/mod.rs
git commit -m "feat: integrate GenericPairMerger into exec event pipeline with real-time streaming"
```

---

### Task 3: Add `pairs_merged` metric to TaskMetrics

**Files:**
- Modify: `src/daemon/store/mod.rs:19-30` (TaskMetrics struct)

**Interfaces:**
- Consumes: nothing new
- Produces: `TaskMetrics.pairs_merged: u64` field, serialized in JSONL, displayed in stats

- [ ] **Step 1: Add field to TaskMetrics**

In `src/daemon/store/mod.rs`, find the `TaskMetrics` struct (lines 19-30). Add after `hints_attached`:

```rust
pub struct TaskMetrics {
    pub raw_output_bytes: u64,
    pub structured_events_bytes: u64,
    pub agent_visible_events: u64,
    pub agent_skipped_events: u64,
    pub locations_extracted: u64,
    pub codes_extracted: u64,
    pub contexts_enriched: u64,
    pub hints_attached: u64,
    pub pairs_merged: u64,  // ← new
}
```

- [ ] **Step 2: Check if stats display needs updating**

Read `src/daemon/store/schema.rs` (around lines 58-65) to see if `TaskMetrics` fields are enumerated for stats aggregation. If `pairs_merged` needs to be added to the aggregate stats, add it following the existing pattern.

Also check `src/daemon/analytics.rs` if it displays per-task metrics. Add `pairs_merged` to the output if relevant.

- [ ] **Step 3: Run cargo build**

Run: `cargo build 2>&1`
Expected: Compiles. The `#[serde(default)]` on TaskMetrics ensures backward compatibility with existing JSONL files that don't have this field.

- [ ] **Step 4: Run unit tests**

Run: `cargo test --lib --bin arshy --bin arshyd 2>&1`
Expected: All tests pass. Existing JSONL files use `#[serde(default)]` so missing `pairs_merged` defaults to 0.

- [ ] **Step 5: Commit**

```bash
git add src/daemon/store/mod.rs src/daemon/store/schema.rs src/daemon/analytics.rs
git commit -m "feat: add pairs_merged metric to TaskMetrics"
```

---

### Task 4: Regenerate fixture tests

**Files:**
- Regenerate: `parsers/builtin/tests/cargo/*.json`
- Regenerate: `parsers/builtin/tests/python/*.json`
- Possibly regenerate: `parsers/builtin/tests/{clippy,docker,terraform,oxlint,helm,git}/*.json`

**Interfaces:**
- Consumes: `pair_merger::merge_diagnostic_location_pairs()` (from Task 1)
- Produces: Updated expected JSON with merged events

- [ ] **Step 1: Run bless mode for all affected parsers**

Run: `ARSHY_BLESS=1 cargo test --bin arshyd 2>&1`
Expected: All fixture tests regenerate expected JSON files. Some tests may fail initially if the fixture test harness doesn't yet call the pair merger — see Step 2.

- [ ] **Step 2: Integrate pair merger into fixture test harness**

If the bless run doesn't apply the pair merger (because the test harness calls `session.parse_line()` directly without the exec pipeline), add the pair merger call to the test harness.

In `src/daemon/parser/mod.rs`, find `run_fixture()` (line 716). After collecting events from `session.parse_line()`, apply the pair merger:

```rust
// Existing: collect events from parse_line
let mut all_events: Vec<TaskEvent> = Vec::new();
for line in lines {
    let events = session.parse_line(line, seq, tool.as_ref());
    all_events.extend(events);
    seq += 1; // adjust as needed
}

// New: apply pair merger
let (merged_events, _pairs_merged) = pair_merger::merge_diagnostic_location_pairs(all_events);
all_events = merged_events;
```

Check the exact code structure of `run_fixture()` and integrate accordingly.

- [ ] **Step 3: Re-run bless mode**

Run: `ARSHY_BLESS=1 cargo test --bin arshyd 2>&1`
Expected: All fixtures regenerate with merged events.

- [ ] **Step 4: Inspect cargo compile-error.json**

Read `parsers/builtin/tests/cargo/compile-error.json`. Verify the merged structure:

**Before (6 events):**
```json
[
  { "type": "diagnostic", "code": "E0308", "message": "mismatched types", "severity": "error" },
  { "type": "location", "file": "src/main.rs", "line": 10, "severity": "info" },
  { "type": "diagnostic", "message": "unused variable: `x`", "severity": "warning" },
  { "type": "location", "file": "src/lib.rs", "line": 22, "severity": "info" },
  { "type": "diagnostic", "code": "E0425", "message": "cannot find value `foo`", "severity": "error" },
  { "type": "location", "file": "src/handler.rs", "line": 45, "severity": "info" }
]
```

**After (3 events):**
```json
[
  { "type": "diagnostic", "code": "E0308", "message": "mismatched types", "severity": "error", "file": "src/main.rs", "line": 10 },
  { "type": "diagnostic", "message": "unused variable: `x`", "severity": "warning", "file": "src/lib.rs", "line": 22 },
  { "type": "diagnostic", "code": "E0425", "message": "cannot find value `foo`", "severity": "error", "file": "src/handler.rs", "line": 45 }
]
```

- [ ] **Step 5: Inspect python pytest-failures.json**

Read `parsers/builtin/tests/python/pytest-failures.json`. Verify:

**Before (7 events):**
```json
[
  { "type": "location", "file": "tests/test_login.py", "line": 15 },
  { "type": "diagnostic", "message": "AssertionError: expected 200, got 500" },
  { "type": "test_result", "file": "tests/test_login.py::test_login_success" },
  { "type": "location", "file": "tests/test_api.py", "line": 42 },
  { "type": "diagnostic", "message": "KeyError: 'user_id'" },
  { "type": "test_result", "file": "tests/test_api.py::test_get_user" },
  { "type": "summary", "message": "...2 failed, 1 passed..." }
]
```

**After (5 events):**
```json
[
  { "type": "diagnostic", "message": "AssertionError: expected 200, got 500", "severity": "error", "file": "tests/test_login.py", "line": 15 },
  { "type": "test_result", "file": "tests/test_login.py::test_login_success" },
  { "type": "diagnostic", "message": "KeyError: 'user_id'", "severity": "error", "file": "tests/test_api.py", "line": 42 },
  { "type": "test_result", "file": "tests/test_api.py::test_get_user" },
  { "type": "summary", "message": "...2 failed, 1 passed..." }
]
```

- [ ] **Step 6: Run all fixture tests**

Run: `cargo test --bin arshyd 2>&1`
Expected: All 37 parser fixture tests pass with ≥95% field match rate.

- [ ] **Step 7: Commit**

```bash
git add parsers/builtin/tests/
git commit -m "test: regenerate fixture tests with merged diagnostic+location events"
```

---

### Task 5: Full validation

**Files:** None (validation only)

- [ ] **Step 1: Run full test suite**

Run: `cargo test --lib --bin arshy --bin arshyd 2>&1`
Expected: All tests pass.

- [ ] **Step 2: Run clippy and format check**

Run: `cargo clippy --all-targets -- -D warnings 2>&1 && cargo fmt --all -- --check 2>&1`
Expected: Zero warnings, format OK.

- [ ] **Step 3: Run doc build**

Run: `cargo doc --no-deps --document-private-items 2>&1`
Expected: No warnings.

- [ ] **Step 4: Integration test with real command**

Run the daemon and execute a cargo build through arshy:

```bash
cargo build --bin arshyd && cargo build --bin arshy
# Kill existing daemon if running
pkill arshyd 2>/dev/null; sleep 1
# Start daemon
./target/debug/arshyd &
sleep 2
# Run a command that produces diagnostic+location pairs
./target/debug/arshy run "cargo build 2>&1" --format json 2>&1 | head -100
```

Verify that the `events` array in the JSON response contains merged events (diagnostic events with `location` field attached).

- [ ] **Step 5: Verify stats show pairs_merged**

Run: `./target/debug/arshy stats 2>&1`
Expected: Stats output includes `pairs_merged` metric (may be 0 if no recent tasks with pairs).

- [ ] **Step 6: Commit (if any fixes needed)**

```bash
git add -A
git commit -m "fix: address validation findings for GenericPairMerger"
```
