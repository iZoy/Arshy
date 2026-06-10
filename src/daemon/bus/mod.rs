//! EventBus — tokio broadcast channel for daemon → proxy notifications.

pub mod router;

use tokio::sync::broadcast;

/// A bus event carrying task updates, diagnostics, or lifecycle signals.
#[derive(Debug, Clone)]
pub struct BusEvent {
    #[allow(dead_code)] // future: multi-connection tracking
    pub connection_id: u64,
    pub kind: BusEventKind,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // Diagnostic contains TaskEvent; boxing adds indirection cost
pub enum BusEventKind {
    TaskUpdate {
        task_id: String,
        status: String,
        elapsed_ms: u64,
    },
    TaskComplete {
        task_id: String,
        exit_code: i32,
        duration_ms: u64,
    },
    Diagnostic {
        task_id: String,
        event: arshy_lib::ipc::TaskEvent,
    },
    #[allow(dead_code)] // future: graceful shutdown notification
    DaemonShutdown {
        reason: String,
        grace_period_ms: u64,
    },
    /// Reserved: stream real-time output for `tail -f` / interactive PTY.
    /// Not currently produced by any code path.
    #[allow(dead_code)]
    StreamOutput {
        task_id: String,
        data: String,
    },
}

/// Multi-producer, multi-consumer event bus.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<BusEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    /// Create a new subscriber receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<BusEvent> {
        self.tx.subscribe()
    }

    /// Publish an event to all subscribers. Best-effort (no error if no receivers).
    pub fn publish(&self, event: BusEvent) {
        let _ = self.tx.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_update(task_id: &str) -> BusEvent {
        BusEvent {
            connection_id: 0,
            kind: BusEventKind::TaskUpdate {
                task_id: task_id.to_string(),
                status: "running".into(),
                elapsed_ms: 100,
            },
        }
    }

    fn make_complete(task_id: &str, exit_code: i32) -> BusEvent {
        BusEvent {
            connection_id: 0,
            kind: BusEventKind::TaskComplete {
                task_id: task_id.to_string(),
                exit_code,
                duration_ms: 500,
            },
        }
    }

    fn make_shutdown(reason: &str) -> BusEvent {
        BusEvent {
            connection_id: 0,
            kind: BusEventKind::DaemonShutdown {
                reason: reason.to_string(),
                grace_period_ms: 3000,
            },
        }
    }

    #[tokio::test]
    async fn publish_subscribe_single() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(make_update("t1"));

        let event = rx.recv().await.unwrap();
        match &event.kind {
            BusEventKind::TaskUpdate { task_id, .. } => assert_eq!(task_id, "t1"),
            _ => panic!("expected TaskUpdate"),
        }
    }

    #[tokio::test]
    async fn publish_subscribe_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.publish(make_complete("t2", 0));

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();

        match &e1.kind {
            BusEventKind::TaskComplete { task_id, exit_code, .. } => {
                assert_eq!(task_id, "t2");
                assert_eq!(*exit_code, 0);
            }
            _ => panic!("expected TaskComplete"),
        }
        match &e2.kind {
            BusEventKind::TaskComplete { task_id, .. } => assert_eq!(task_id, "t2"),
            _ => panic!("expected TaskComplete"),
        }
    }

    #[tokio::test]
    async fn publish_no_subscribers_does_not_panic() {
        let bus = EventBus::new();
        // No subscribers — publish should silently succeed
        bus.publish(make_update("t3"));
    }

    #[tokio::test]
    async fn publish_multiple_events_sequentially() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(make_update("a"));
        bus.publish(make_update("b"));
        bus.publish(make_shutdown("graceful"));

        let e1 = rx.recv().await.unwrap();
        let e2 = rx.recv().await.unwrap();
        let e3 = rx.recv().await.unwrap();

        match &e1.kind {
            BusEventKind::TaskUpdate { task_id, .. } => assert_eq!(task_id, "a"),
            _ => panic!("expected TaskUpdate"),
        }
        match &e2.kind {
            BusEventKind::TaskUpdate { task_id, .. } => assert_eq!(task_id, "b"),
            _ => panic!("expected TaskUpdate"),
        }
        match &e3.kind {
            BusEventKind::DaemonShutdown { reason, .. } => assert_eq!(reason, "graceful"),
            _ => panic!("expected DaemonShutdown"),
        }
    }

    #[tokio::test]
    async fn subscriber_late_join_misses_earlier_events() {
        let bus = EventBus::new();

        bus.publish(make_update("early"));

        // Subscribe AFTER the event — should not receive it
        let mut rx = bus.subscribe();
        assert!(rx.try_recv().is_err());
    }
}
