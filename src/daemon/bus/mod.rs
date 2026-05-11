//! EventBus — tokio broadcast channel for daemon → proxy notifications.

use tokio::sync::broadcast;

/// A bus event carrying task updates, diagnostics, or lifecycle signals.
#[derive(Debug, Clone)]
pub struct BusEvent {
    pub connection_id: u64,
    pub kind: BusEventKind,
}

#[derive(Debug, Clone)]
pub enum BusEventKind {
    TaskUpdate { task_id: String, status: String, elapsed_ms: u64 },
    TaskComplete { task_id: String, exit_code: i32, duration_ms: u64 },
    Diagnostic { task_id: String, event: arshy_lib::ipc::TaskEvent },
    DaemonShutdown { reason: String, grace_period_ms: u64 },
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
