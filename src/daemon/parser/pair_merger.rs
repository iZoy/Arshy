//! Generic diagnostic-location pair merger.
//!
//! Some tools (e.g. cargo, Python traceback) emit a diagnostic and its location
//! as two separate consecutive events. This module merges them into a single
//! diagnostic event with location attached.
//!
//! Supports both streaming (`feed()`/`finish()`) and batch (`merge_diagnostic_location_pairs()`).

use crate::ipc::TaskEvent;

/// Merges consecutive diagnostic + location events into single diagnostics.
///
/// Buffers one pending event. On each `feed()` call, determines whether
/// the current event should be merged with the pending one or flushed.
pub struct GenericPairMerger {
    /// Buffer for the last event that may receive a merged partner.
    pending: Option<TaskEvent>,
    /// Counter of merged pairs (for observability).
    merged_count: u64,
}

impl GenericPairMerger {
    pub fn new() -> Self {
        Self { pending: None, merged_count: 0 }
    }

    /// Total pairs merged so far.
    pub fn merged_count(&self) -> u64 {
        self.merged_count
    }

    /// Feed an event through the merger. Returns `Some(event)` when an event
    /// is ready to be emitted; returns `None` if the event was absorbed into
    /// the pending buffer.
    pub fn feed(&mut self, event: TaskEvent) -> Option<TaskEvent> {
        let current_is_diag = is_diagnostic(&event.event_type);
        let current_is_loc = is_location(&event.event_type);

        // If current is neither diagnostic nor location, handle pending then
        // pass current through.
        if !current_is_diag && !current_is_loc {
            let flushed = self.pending.take();
            if flushed.is_some() {
                // Swap: store current in pending, return old pending.
                self.pending = Some(event);
                return flushed;
            }
            // No pending — return current directly.
            return Some(event);
        }

        let pending_is_diag = self.pending.as_ref().is_some_and(|p| is_diagnostic(&p.event_type));
        let pending_is_loc = self.pending.as_ref().is_some_and(|p| is_location(&p.event_type));

        if current_is_loc && pending_is_diag {
            // Forward merge: location follows diagnostic.
            let mut diag = self.pending.take().unwrap();
            diag = merge_location(diag, &event);
            self.merged_count += 1;
            Some(diag)
        } else if current_is_diag && pending_is_loc {
            // Backward merge: diagnostic follows location.
            let loc = self.pending.take().unwrap();
            let mut diag = event;
            diag = merge_location(diag, &loc);
            self.merged_count += 1;
            Some(diag)
        } else if current_is_loc && pending_is_loc {
            // Two locations in a row — flush the first, buffer the second.
            self.pending.replace(event)
        } else {
            // Two diagnostics in a row — flush the first, buffer the second.
            self.pending.replace(event)
        }
    }

    /// Flush any remaining buffered event (call at end of stream).
    pub fn finish(&mut self) -> Option<TaskEvent> {
        self.pending.take()
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
        if let Some(emitted) = merger.feed(event) {
            result.push(emitted);
        }
    }
    if let Some(leftover) = merger.finish() {
        result.push(leftover);
    }

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
    fn feed_streaming_flush_on_non_pairable() {
        let mut merger = GenericPairMerger::new();
        // Feed diagnostic — buffered
        let result = merger.feed(diag(1, "error"));
        assert!(result.is_none());
        // Feed summary — should flush the diagnostic, buffer the summary
        let result = merger.feed(summary(2, "done"));
        assert!(result.is_some());
        let flushed = result.unwrap();
        assert_eq!(flushed.event_type, "diagnostic");
        assert_eq!(flushed.message, "error");
        // Finish — should return the buffered summary
        let result = merger.finish();
        assert!(result.is_some());
        let leftover = result.unwrap();
        assert_eq!(leftover.event_type, "summary");
        assert_eq!(leftover.message, "done");
        assert_eq!(merger.merged_count(), 0);
    }
}
