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
            // Reserved: not currently produced, no notification mapping yet
            BusEventKind::StreamOutput { .. } => return None,
        };

        Some(ipc::Notification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::ipc::TaskEvent;

    fn make_bus_event(kind: BusEventKind) -> BusEvent {
        BusEvent { connection_id: 0, kind }
    }

    #[test]
    fn router_task_update() {
        let event = make_bus_event(BusEventKind::TaskUpdate {
            task_id: "t1".into(),
            status: "running".into(),
            elapsed_ms: 200,
        });
        let notif = NotificationRouter::to_notification(&event).unwrap();
        assert_eq!(notif.method, ipc::NOTIF_TASK_UPDATE);
        assert_eq!(notif.params["task_id"], "t1");
        assert_eq!(notif.params["status"], "running");
        assert_eq!(notif.params["elapsed_ms"], 200);
    }

    #[test]
    fn router_task_complete_success() {
        let event = make_bus_event(BusEventKind::TaskComplete {
            task_id: "t2".into(),
            exit_code: 0,
            duration_ms: 1500,
        });
        let notif = NotificationRouter::to_notification(&event).unwrap();
        assert_eq!(notif.method, ipc::NOTIF_TASK_COMPLETE);
        assert_eq!(notif.params["exit_code"], 0);
        assert_eq!(notif.params["duration_ms"], 1500);
    }

    #[test]
    fn router_task_complete_failure() {
        let event = make_bus_event(BusEventKind::TaskComplete {
            task_id: "t3".into(),
            exit_code: 1,
            duration_ms: 300,
        });
        let notif = NotificationRouter::to_notification(&event).unwrap();
        assert_eq!(notif.params["exit_code"], 1);
    }

    #[test]
    fn router_diagnostic() {
        let task_event = TaskEvent {
            seq: 1,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: Some("E0308".into()),
            message: "type mismatch".into(),
            location: None,
            context: None,
        };
        let event = make_bus_event(BusEventKind::Diagnostic {
            task_id: "t4".into(),
            event: task_event,
        });
        let notif = NotificationRouter::to_notification(&event).unwrap();
        assert_eq!(notif.method, ipc::NOTIF_DIAGNOSTIC);
        assert_eq!(notif.params["task_id"], "t4");
        assert_eq!(notif.params["event"]["type"], "diagnostic");
        assert_eq!(notif.params["event"]["severity"], "error");
        assert_eq!(notif.params["event"]["code"], "E0308");
    }

    #[test]
    fn router_daemon_shutdown() {
        let event = make_bus_event(BusEventKind::DaemonShutdown {
            reason: "signal".into(),
            grace_period_ms: 3000,
        });
        let notif = NotificationRouter::to_notification(&event).unwrap();
        assert_eq!(notif.method, ipc::NOTIF_DAEMON_SHUTDOWN);
        assert_eq!(notif.params["reason"], "signal");
        assert_eq!(notif.params["grace_period_ms"], 3000);
    }

    #[test]
    fn router_notification_has_jsonrpc_version() {
        let event = make_bus_event(BusEventKind::TaskUpdate {
            task_id: "t5".into(),
            status: "running".into(),
            elapsed_ms: 0,
        });
        let notif = NotificationRouter::to_notification(&event).unwrap();
        assert_eq!(notif.jsonrpc, "2.0");
    }
}
