use arshy_lib::ipc;
use super::{BusEvent, BusEventKind};

/// Maps daemon bus events to JSON-RPC notifications for proxy delivery.
pub struct NotificationRouter;

impl NotificationRouter {
    /// Convert a BusEvent into an IPC Notification suitable for the wire.
    pub fn to_notification(event: &BusEvent) -> Option<ipc::Notification> {
        let (method, params) = match &event.kind {
            BusEventKind::TaskUpdate { task_id, status, elapsed_ms } => {
                let payload = serde_json::json!({
                    "task_id": task_id,
                    "status": status,
                    "elapsed_ms": elapsed_ms,
                });
                (ipc::NOTIF_TASK_UPDATE, payload)
            }
            BusEventKind::TaskComplete { task_id, exit_code, duration_ms } => {
                let payload = serde_json::json!({
                    "task_id": task_id,
                    "exit_code": exit_code,
                    "duration_ms": duration_ms,
                });
                (ipc::NOTIF_TASK_COMPLETE, payload)
            }
            BusEventKind::Diagnostic { task_id, event } => {
                let payload = serde_json::json!({
                    "task_id": task_id,
                    "event": event,
                });
                (ipc::NOTIF_DIAGNOSTIC, payload)
            }
            BusEventKind::DaemonShutdown { reason, grace_period_ms } => {
                let payload = serde_json::json!({
                    "reason": reason,
                    "grace_period_ms": grace_period_ms,
                });
                (ipc::NOTIF_DAEMON_SHUTDOWN, payload)
            }
        };

        Some(ipc::Notification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        })
    }
}
