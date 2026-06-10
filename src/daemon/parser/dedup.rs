//! Event deduplicator — collapses consecutive identical events.
//!
//! Tracks consecutive events with the same event_type and message,
//! emitting a single event with "(repeated N times)" suffix when
//! duplicates are flushed.

use arshy_lib::ipc::TaskEvent;

pub struct Deduplicator {
    last_event_type: Option<String>,
    last_message: Option<String>,
    last_severity: Option<String>,
    last_location: Option<arshy_lib::ipc::EventLocation>,
    last_code: Option<String>,
    last_context: Option<arshy_lib::ipc::EventContext>,
    repeat_count: u64,
    first_seq: u64,
}

impl Deduplicator {
    pub fn new() -> Self {
        Self {
            last_event_type: None,
            last_message: None,
            last_severity: None,
            last_location: None,
            last_code: None,
            last_context: None,
            repeat_count: 0,
            first_seq: 0,
        }
    }

    /// Feed an event. Returns Some if the event should be stored (either
    /// a new non-duplicate, or a flushed accumulated duplicate).
    /// Returns None if the event is a duplicate (accumulated internally).
    pub fn feed(&mut self, event: TaskEvent) -> Option<TaskEvent> {
        let is_dup = self.last_event_type.as_deref() == Some(&event.event_type)
            && self.last_message.as_deref() == Some(&event.message)
            && self.last_location == event.location;

        if is_dup {
            self.repeat_count += 1;
            return None;
        }

        let flushed = self.flush();
        self.first_seq = event.seq;
        self.last_event_type = Some(event.event_type.clone());
        self.last_message = Some(event.message.clone());
        self.last_severity = event.severity.clone();
        self.last_location = event.location.clone();
        self.last_code = event.code.clone();
        self.last_context = event.context.clone();
        self.repeat_count = 1;

        flushed.or(Some(event))
    }

    /// Flush remaining accumulated event at end of stream.
    pub fn finish(&mut self) -> Option<TaskEvent> {
        self.flush()
    }

    fn flush(&mut self) -> Option<TaskEvent> {
        if self.repeat_count <= 1 {
            return None;
        }
        let message = format!(
            "{} (repeated {} times)",
            self.last_message.as_deref().unwrap_or(""),
            self.repeat_count
        );
        let event = TaskEvent {
            seq: self.first_seq,
            event_type: self.last_event_type.clone().unwrap_or_default(),
            severity: self.last_severity.clone(),
            code: self.last_code.clone(),
            message,
            location: self.last_location.clone(),
            context: self.last_context.clone(),
        };
        self.last_event_type = None;
        self.last_message = None;
        self.last_severity = None;
        self.last_location = None;
        self.last_code = None;
        self.last_context = None;
        self.repeat_count = 0;
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(event_type: &str, severity: &str, message: &str, seq: u64) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: event_type.into(),
            severity: Some(severity.into()),
            code: None,
            message: message.into(),
            location: None,
            context: None,
        }
    }

    #[test]
    fn single_event_passthrough() {
        let mut d = Deduplicator::new();
        let result = d.feed(make_event("log", "info", "hello", 0));
        assert!(result.is_some());
        assert_eq!(result.unwrap().message, "hello");
        assert!(d.finish().is_none());
    }

    #[test]
    fn consecutive_duplicates_collapsed() {
        let mut d = Deduplicator::new();
        assert!(d.feed(make_event("log", "warning", "unused x", 0)).is_some());
        assert!(d.feed(make_event("log", "warning", "unused x", 1)).is_none());
        assert!(d.feed(make_event("log", "warning", "unused x", 2)).is_none());
        let flushed = d.feed(make_event("log", "error", "bad", 3));
        assert!(flushed.is_some());
        let flushed = flushed.unwrap();
        assert_eq!(flushed.message, "unused x (repeated 3 times)");
        assert_eq!(flushed.severity.as_deref(), Some("warning"));
    }

    #[test]
    fn finish_flushes_remaining() {
        let mut d = Deduplicator::new();
        d.feed(make_event("log", "info", "repeat me", 0));
        d.feed(make_event("log", "info", "repeat me", 1));
        d.feed(make_event("log", "info", "repeat me", 2));
        let result = d.finish();
        assert!(result.is_some());
        assert_eq!(result.unwrap().message, "repeat me (repeated 3 times)");
    }

    #[test]
    fn non_consecutive_not_collapsed() {
        let mut d = Deduplicator::new();
        let r1 = d.feed(make_event("log", "info", "a", 0));
        assert!(r1.is_some());
        let r2 = d.feed(make_event("log", "info", "b", 1));
        assert!(r2.is_some());
        let r3 = d.feed(make_event("log", "info", "a", 2));
        assert!(r3.is_some());
        assert!(d.finish().is_none());
    }

    #[test]
    fn two_duplicates_flushed_on_finish() {
        let mut d = Deduplicator::new();
        d.feed(make_event("diagnostic", "error", "fail", 0));
        d.feed(make_event("diagnostic", "error", "fail", 1));
        let result = d.finish().expect("should flush");
        assert_eq!(result.message, "fail (repeated 2 times)");
        assert_eq!(result.severity.as_deref(), Some("error"));
    }

    #[test]
    fn different_types_not_collapsed() {
        let mut d = Deduplicator::new();
        d.feed(make_event("log", "info", "msg", 0));
        let r = d.feed(make_event("diagnostic", "error", "msg", 1));
        assert!(r.is_some());
    }

    #[test]
    fn same_type_message_different_location_not_collapsed() {
        use arshy_lib::ipc::EventLocation;

        let mut d = Deduplicator::new();
        let e1 = TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "unused variable".into(),
            location: Some(EventLocation { file: "src/main.rs".into(), line: 10, column: None }),
            context: None,
        };
        let e2 = TaskEvent {
            seq: 1,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "unused variable".into(),
            location: Some(EventLocation { file: "src/lib.rs".into(), line: 42, column: Some(5) }),
            context: None,
        };
        assert!(d.feed(e1).is_some());
        assert!(d.feed(e2).is_some());
        assert!(d.finish().is_none());
    }

    #[test]
    fn flush_preserves_context() {
        use arshy_lib::ipc::EventContext;

        let mut d = Deduplicator::new();
        let ctx = Some(EventContext {
            before: vec!["fn foo() {".into()],
            line: "    let x = 1;".into(),
            after: vec!["}".into()],
        });
        let e1 = TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "unused variable".into(),
            location: None,
            context: ctx.clone(),
        };
        let e2 = TaskEvent {
            seq: 1,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "unused variable".into(),
            location: None,
            context: None,
        };
        d.feed(e1);
        d.feed(e2);
        let flushed = d.finish().expect("should flush");
        assert_eq!(flushed.context, ctx);
    }
}
