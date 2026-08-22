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
static CARRIER_SHELL: AtomicU64 = AtomicU64::new(0);
static CARRIER_SHELL_COMPOSITE: AtomicU64 = AtomicU64::new(0);
static CARRIER_PYTHON: AtomicU64 = AtomicU64::new(0);
static CARRIER_SCRIPT_OTHER: AtomicU64 = AtomicU64::new(0);
static CARRIER_UNKNOWN: AtomicU64 = AtomicU64::new(0);

pub fn record_task_created() {
    TASKS_CREATED.fetch_add(1, Ordering::Relaxed);
}
pub fn record_task_completed(success: bool) {
    TASKS_COMPLETED.fetch_add(1, Ordering::Relaxed);
    if !success {
        TASKS_FAILED.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn record_event_emitted() {
    EVENTS_EMITTED.fetch_add(1, Ordering::Relaxed);
}
pub fn record_connection_accepted() {
    CONNECTIONS_ACCEPTED.fetch_add(1, Ordering::Relaxed);
}

/// Record the execution carrier of one command (Q1: shell / shell_composite
/// / python / script_other / unknown). Counts every command, short or long.
pub fn record_carrier(carrier: &str) {
    match carrier {
        "shell" => CARRIER_SHELL.fetch_add(1, Ordering::Relaxed),
        "shell_composite" => CARRIER_SHELL_COMPOSITE.fetch_add(1, Ordering::Relaxed),
        "python" => CARRIER_PYTHON.fetch_add(1, Ordering::Relaxed),
        "script_other" => CARRIER_SCRIPT_OTHER.fetch_add(1, Ordering::Relaxed),
        _ => CARRIER_UNKNOWN.fetch_add(1, Ordering::Relaxed),
    };
}

/// Snapshot of all counters for inclusion in stats/health responses.
pub fn snapshot() -> serde_json::Value {
    serde_json::json!({
        "tasks_created": TASKS_CREATED.load(Ordering::Relaxed),
        "tasks_completed": TASKS_COMPLETED.load(Ordering::Relaxed),
        "tasks_failed": TASKS_FAILED.load(Ordering::Relaxed),
        "events_emitted": EVENTS_EMITTED.load(Ordering::Relaxed),
        "connections_accepted": CONNECTIONS_ACCEPTED.load(Ordering::Relaxed),
        "carrier_distribution": {
            "shell": CARRIER_SHELL.load(Ordering::Relaxed),
            "shell_composite": CARRIER_SHELL_COMPOSITE.load(Ordering::Relaxed),
            "python": CARRIER_PYTHON.load(Ordering::Relaxed),
            "script_other": CARRIER_SCRIPT_OTHER.load(Ordering::Relaxed),
            "unknown": CARRIER_UNKNOWN.load(Ordering::Relaxed),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_has_expected_keys() {
        let s = snapshot();
        for key in [
            "tasks_created",
            "tasks_completed",
            "tasks_failed",
            "events_emitted",
            "connections_accepted",
            "carrier_distribution",
        ] {
            assert!(s.get(key).is_some(), "snapshot missing key {key}");
        }
    }

    #[test]
    fn record_task_created_increments() {
        let before = snapshot().get("tasks_created").and_then(|v| v.as_u64()).unwrap();
        record_task_created();
        let after = snapshot().get("tasks_created").and_then(|v| v.as_u64()).unwrap();
        assert!(after > before);
    }

    #[test]
    fn record_task_completed_success_does_not_increment_failed() {
        let failed_before = snapshot().get("tasks_failed").and_then(|v| v.as_u64()).unwrap();
        let completed_before = snapshot().get("tasks_completed").and_then(|v| v.as_u64()).unwrap();
        record_task_completed(true);
        let completed_after = snapshot().get("tasks_completed").and_then(|v| v.as_u64()).unwrap();
        let failed_after = snapshot().get("tasks_failed").and_then(|v| v.as_u64()).unwrap();
        assert!(completed_after > completed_before);
        assert!(failed_after >= failed_before);
    }

    #[test]
    fn record_task_completed_failure_increments_failed() {
        let failed_before = snapshot().get("tasks_failed").and_then(|v| v.as_u64()).unwrap();
        record_task_completed(false);
        let failed_after = snapshot().get("tasks_failed").and_then(|v| v.as_u64()).unwrap();
        assert!(failed_after > failed_before);
    }

    #[test]
    fn record_event_emitted_increments() {
        let before = snapshot().get("events_emitted").and_then(|v| v.as_u64()).unwrap();
        record_event_emitted();
        let after = snapshot().get("events_emitted").and_then(|v| v.as_u64()).unwrap();
        assert!(after > before);
    }

    #[test]
    fn record_connection_accepted_increments() {
        let before = snapshot().get("connections_accepted").and_then(|v| v.as_u64()).unwrap();
        record_connection_accepted();
        let after = snapshot().get("connections_accepted").and_then(|v| v.as_u64()).unwrap();
        assert!(after > before);
    }

    #[test]
    fn record_carrier_increments_distribution() {
        let before = snapshot()["carrier_distribution"]["python"].as_u64().unwrap();
        record_carrier("python");
        let after = snapshot()["carrier_distribution"]["python"].as_u64().unwrap();
        assert!(after > before);
    }
}
