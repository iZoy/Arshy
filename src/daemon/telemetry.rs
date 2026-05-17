//! Lightweight telemetry counters — zero-allocation atomic metrics.
//!
//! These counters are incremented by the executor and bus, and exposed
//! through the `daemon/stats` and `daemon/health` IPC methods.
//!
//! Uses `std::sync::atomic` to avoid contention; no external metrics
//! dependencies required.

use std::sync::atomic::{AtomicU64, Ordering};

static TASKS_CREATED: AtomicU64 = AtomicU64::new(0);
static TASKS_COMPLETED: AtomicU64 = AtomicU64::new(0);
static TASKS_FAILED: AtomicU64 = AtomicU64::new(0);
static EVENTS_EMITTED: AtomicU64 = AtomicU64::new(0);
static CONNECTIONS_ACCEPTED: AtomicU64 = AtomicU64::new(0);

pub fn record_task_created() { TASKS_CREATED.fetch_add(1, Ordering::Relaxed); }
pub fn record_task_completed(success: bool) {
    TASKS_COMPLETED.fetch_add(1, Ordering::Relaxed);
    if !success { TASKS_FAILED.fetch_add(1, Ordering::Relaxed); }
}
#[allow(dead_code)]
pub fn record_event_emitted() { EVENTS_EMITTED.fetch_add(1, Ordering::Relaxed); }
pub fn record_connection_accepted() { CONNECTIONS_ACCEPTED.fetch_add(1, Ordering::Relaxed); }

/// Snapshot of all counters for inclusion in stats/health responses.
pub fn snapshot() -> serde_json::Value {
    serde_json::json!({
        "tasks_created": TASKS_CREATED.load(Ordering::Relaxed),
        "tasks_completed": TASKS_COMPLETED.load(Ordering::Relaxed),
        "tasks_failed": TASKS_FAILED.load(Ordering::Relaxed),
        "events_emitted": EVENTS_EMITTED.load(Ordering::Relaxed),
        "connections_accepted": CONNECTIONS_ACCEPTED.load(Ordering::Relaxed),
    })
}
