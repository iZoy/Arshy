//! Generic diagnostic-location pair merger.
//!
//! Some tools (e.g. cargo, Python traceback) emit a diagnostic and its location
//! as two separate consecutive events. This module merges them into a single
//! diagnostic event with location attached.
//!
//! Supports both streaming (`feed()`/`finish()`) and batch (`merge_diagnostic_location_pairs()`).

use crate::ipc::TaskEvent;
use std::collections::VecDeque;

/// Merges consecutive diagnostic + location events into single diagnostics.
///
/// Buffers one pending event. On each `feed()` call, determines whether
/// the current event should be merged with the pending one or flushed.
pub struct GenericPairMerger {
    /// Buffer for the last event that may receive a merged partner.
    pending: Option<TaskEvent>,
    /// Counter of merged pairs (for observability).
    merged_count: u64,
    ready: VecDeque<TaskEvent>,
}

impl GenericPairMerger {
    pub fn new() -> Self {
        Self { pending: None, merged_count: 0, ready: VecDeque::new() }
    }

    /// Total pairs merged so far.
    pub fn merged_count(&self) -> u64 {
        self.merged_count
    }

    /// Feed an event through the merger. Returns `Some(event)` when an event
    /// is ready to be emitted; returns `None` if the event was absorbed into
    /// the pending buffer.
    ///
    /// Handles diagnostic+location pairs in both orderings (forward and backward),
    /// even when non-pairable events (log, summary, etc.) appear between them.
    pub fn feed(&mut self, event: TaskEvent) -> Option<TaskEvent> {
        let mut emitted = self.feed_all(event);
        let first = emitted.first().cloned();
        if !emitted.is_empty() {
            emitted.remove(0);
        }
        // Preserve any additional events for the next call. Production code
        // uses feed_all directly; this compatibility wrapper is retained for
        // focused streaming callers and older tests.
        if !emitted.is_empty() {
            self.ready.extend(emitted);
        }
        first
    }

    /// Feed an event and return every event made ready by the operation.
    ///
    /// Keeping this vector-valued form is important when a non-pairable event
    /// arrives after a buffered diagnostic: both the diagnostic and the new
    /// event must be emitted without reordering or dropping either one.
    pub fn feed_all(&mut self, event: TaskEvent) -> Vec<TaskEvent> {
        if is_diagnostic(&event.event_type) {
            if let Some(pending) = self.pending.take() {
                if is_location(&pending.event_type) {
                    // Backward merge: pending location + current diagnostic
                    let mut diag = event;
                    diag = merge_location(diag, &pending);
                    self.merged_count += 1;
                    self.ready.push_back(diag);
                } else {
                    // Only adjacent diagnostic pairs are meaningful. Flush
                    // the previous diagnostic and buffer this one; never
                    // search across summaries/logs for a later location.
                    self.ready.push_back(pending);
                    self.pending = Some(event);
                }
            } else {
                self.pending = Some(event);
            }
        } else if is_location(&event.event_type) {
            if let Some(pending) = self.pending.take() {
                if is_diagnostic(&pending.event_type) {
                    // Forward merge: pending diagnostic + current location
                    let mut diag = pending;
                    diag = merge_location(diag, &event);
                    self.merged_count += 1;
                    self.ready.push_back(diag);
                } else {
                    self.ready.push_back(pending);
                    self.pending = Some(event);
                }
            } else {
                self.pending = Some(event);
            }
        } else if let Some(pending) = self.pending.take() {
            // A non-pairable event terminates adjacency. Preserve both events
            // in source order.
            self.ready.push_back(pending);
            self.ready.push_back(event);
        } else {
            self.ready.push_back(event);
        }

        self.ready.drain(..).collect()
    }

    /// Flush any remaining buffered event (call at end of stream).
    pub fn finish(&mut self) -> Option<TaskEvent> {
        self.finish_all().into_iter().next()
    }

    /// Flush all buffered and ready events in source order.
    pub fn finish_all(&mut self) -> Vec<TaskEvent> {
        if let Some(pending) = self.pending.take() {
            self.ready.push_back(pending);
        }
        self.ready.drain(..).collect()
    }
}

impl Default for GenericPairMerger {
    fn default() -> Self {
        Self::new()
    }
}

/// Merge a location event's location into a diagnostic event.
///
/// Only copies if the diagnostic has no existing location.
fn merge_location(mut diag: TaskEvent, loc_event: &TaskEvent) -> TaskEvent {
    if diag.location.is_none() {
        diag.location = loc_event.location.clone();
    }
    diag
}

fn is_diagnostic(event_type: &str) -> bool {
    event_type == "diagnostic" || event_type == "crash"
}

fn is_location(event_type: &str) -> bool {
    event_type == "location"
}

/// Batch merge: feed all events through a `GenericPairMerger`, collect emitted
/// events, and flush at the end. Returns `(merged_events, merged_count)`.
pub fn merge_diagnostic_location_pairs(events: Vec<TaskEvent>) -> (Vec<TaskEvent>, u64) {
    let mut merger = GenericPairMerger::new();
    let mut result = Vec::new();

    for event in events {
        result.extend(merger.feed_all(event));
    }
    result.extend(merger.finish_all());

    (result, merger.merged_count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::EventLocation;

    fn diag(seq: u64, msg: &str) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: msg.into(),
            location: None,
            context: None,
            hint: None,
        }
    }

    fn diag_with_loc(seq: u64, msg: &str, file: &str, line: u64) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: msg.into(),
            location: Some(EventLocation { file: file.into(), line, column: None }),
            context: None,
            hint: None,
        }
    }

    fn loc(seq: u64, file: &str, line: u64) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: "location".into(),
            severity: None,
            code: None,
            message: String::new(),
            location: Some(EventLocation { file: file.into(), line, column: Some(1) }),
            context: None,
            hint: None,
        }
    }

    fn summary(seq: u64, msg: &str) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: "summary".into(),
            severity: None,
            code: None,
            message: msg.into(),
            location: None,
            context: None,
            hint: None,
        }
    }

    fn log(seq: u64, msg: &str) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: "log".into(),
            severity: Some("info".into()),
            code: None,
            message: msg.into(),
            location: None,
            context: None,
            hint: None,
        }
    }

    // --- Batch tests (8) ---

    #[test]
    fn forward_merge_diagnostic_then_location() {
        let events = vec![diag(1, "type mismatch"), loc(2, "main.rs", 42)];
        let (result, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(result.len(), 1);
        assert_eq!(count, 1);
        assert_eq!(result[0].event_type, "diagnostic");
        let loc = result[0].location.as_ref().unwrap();
        assert_eq!(loc.file, "main.rs");
        assert_eq!(loc.line, 42);
    }

    #[test]
    fn backward_merge_location_then_diagnostic() {
        let events = vec![loc(1, "lib.rs", 10), diag(2, "borrow error")];
        let (result, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(result.len(), 1);
        assert_eq!(count, 1);
        assert_eq!(result[0].event_type, "diagnostic");
        let loc = result[0].location.as_ref().unwrap();
        assert_eq!(loc.file, "lib.rs");
        assert_eq!(loc.line, 10);
    }

    #[test]
    fn consecutive_locations_flush_first() {
        let events = vec![diag(1, "error A"), loc(2, "a.rs", 5), loc(3, "b.rs", 10)];
        let (result, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(result.len(), 2);
        assert_eq!(count, 1);
        // First: diag merged with first location
        assert_eq!(result[0].event_type, "diagnostic");
        let loc0 = result[0].location.as_ref().unwrap();
        assert_eq!(loc0.file, "a.rs");
        // Second: standalone location
        assert_eq!(result[1].event_type, "location");
        let loc1 = result[1].location.as_ref().unwrap();
        assert_eq!(loc1.file, "b.rs");
    }

    #[test]
    fn consecutive_diagnostics_flush_first() {
        let events = vec![diag(1, "error A"), diag(2, "error B"), loc(3, "c.rs", 20)];
        let (result, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(result.len(), 2);
        assert_eq!(count, 1);
        // First: standalone diag (flushed when second diag arrives)
        assert_eq!(result[0].event_type, "diagnostic");
        assert!(result[0].location.is_none());
        assert_eq!(result[0].message, "error A");
        // Second: merged with location
        assert_eq!(result[1].event_type, "diagnostic");
        let loc1 = result[1].location.as_ref().unwrap();
        assert_eq!(loc1.file, "c.rs");
        assert_eq!(loc1.line, 20);
    }

    #[test]
    fn no_pair_unchanged() {
        let events = vec![diag(1, "error"), summary(2, "2 tests failed")];
        let (result, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(result.len(), 2);
        assert_eq!(count, 0);
        // A non-pairable event terminates adjacency and preserves source order.
        assert_eq!(result[0].event_type, "diagnostic");
        assert_eq!(result[1].event_type, "summary");
    }

    #[test]
    fn empty_input() {
        let (result, count) = merge_diagnostic_location_pairs(vec![]);
        assert_eq!(result.len(), 0);
        assert_eq!(count, 0);
    }

    #[test]
    fn does_not_overwrite_existing_location() {
        let events = vec![diag_with_loc(1, "error", "original.rs", 1), loc(2, "new.rs", 99)];
        let (result, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(result.len(), 1);
        assert_eq!(count, 1);
        let loc = result[0].location.as_ref().unwrap();
        assert_eq!(loc.file, "original.rs");
        assert_eq!(loc.line, 1);
    }

    #[test]
    fn python_multi_frame_traceback() {
        // Pattern: diag + loc + loc + diag
        // Frame 1: diag(1) merged with loc(2) — forward merge
        // loc(3) buffered (pending is now loc)
        // diag(4) merges with loc(3) — backward merge
        let events = vec![
            diag(1, "Traceback: ZeroDivisionError"),
            loc(2, "app.py", 10),
            loc(3, "app.py", 5),
            diag(4, "ValueError"),
        ];
        let (result, count) = merge_diagnostic_location_pairs(events);
        assert_eq!(result.len(), 2);
        assert_eq!(count, 2);
        // First merged event
        assert_eq!(result[0].event_type, "diagnostic");
        let loc0 = result[0].location.as_ref().unwrap();
        assert_eq!(loc0.file, "app.py");
        assert_eq!(loc0.line, 10);
        // Second merged event
        assert_eq!(result[1].event_type, "diagnostic");
        let loc1 = result[1].location.as_ref().unwrap();
        assert_eq!(loc1.file, "app.py");
        assert_eq!(loc1.line, 5);
    }

    // --- Streaming tests (3) ---

    #[test]
    fn feed_streaming_forward_merge() {
        let mut merger = GenericPairMerger::new();
        // Feed diagnostic — should be buffered, returns None
        let result = merger.feed(diag(1, "type mismatch"));
        assert!(result.is_none());
        // Feed location — should merge with buffered diagnostic
        let result = merger.feed(loc(2, "main.rs", 42));
        assert!(result.is_some());
        let merged = result.unwrap();
        assert_eq!(merged.event_type, "diagnostic");
        let loc = merged.location.as_ref().unwrap();
        assert_eq!(loc.file, "main.rs");
        assert_eq!(loc.line, 42);
        // Finish — nothing left
        let result = merger.finish();
        assert!(result.is_none());
        assert_eq!(merger.merged_count(), 1);
    }

    #[test]
    fn feed_streaming_backward_merge() {
        let mut merger = GenericPairMerger::new();
        // Feed location — should be buffered
        let result = merger.feed(loc(1, "lib.rs", 10));
        assert!(result.is_none());
        // Feed diagnostic — backward merge
        let result = merger.feed(diag(2, "borrow error"));
        assert!(result.is_some());
        let merged = result.unwrap();
        assert_eq!(merged.event_type, "diagnostic");
        let loc = merged.location.as_ref().unwrap();
        assert_eq!(loc.file, "lib.rs");
        assert_eq!(loc.line, 10);
        assert_eq!(merger.merged_count(), 1);
    }

    #[test]
    fn feed_streaming_non_pairable_terminates_pair() {
        let mut merger = GenericPairMerger::new();
        // Feed diagnostic — buffered
        let result = merger.feed(diag(1, "error"));
        assert!(result.is_none());
        // Feed summary — both events become ready in source order.
        let result = merger.feed_all(summary(2, "done"));
        assert_eq!(
            result.iter().map(|e| e.event_type.as_str()).collect::<Vec<_>>(),
            vec!["diagnostic", "summary"]
        );
        // A later location must not reach back across the summary.
        assert!(merger.feed(loc(3, "main.rs", 42)).is_none());
        assert_eq!(merger.finish_all().len(), 1);
        assert_eq!(merger.merged_count(), 0);
    }

    #[test]
    fn feed_streaming_context_lines_do_not_pair_across_logs() {
        let mut merger = GenericPairMerger::new();
        // diagnostic — buffered
        assert!(merger.feed(diag(1, "mismatched types")).is_none());
        // Context lines terminate the generic pair; RustcContextMerger handles
        // them in the production pipeline after the location pair is formed.
        let events = merger.feed_all(log(2, "  |"));
        assert_eq!(events[0].event_type, "diagnostic");
        let events = merger.feed_all(log(3, "2 | let x = \"hello\";"));
        assert_eq!(events[0].event_type, "log");
        assert!(merger.feed(loc(4, "main.rs", 2)).is_none());
        assert_eq!(merger.finish_all().len(), 1);
        assert_eq!(merger.merged_count(), 0);
    }
}
