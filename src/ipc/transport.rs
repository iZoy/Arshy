//! UDS transport — connect, send JSON-RPC requests, read responses.

use super::{Notification, Request, Response};
use crate::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Connect to the daemon at the given UDS path.
pub async fn connect(socket_path: &std::path::Path) -> Result<UnixStream> {
    Ok(UnixStream::connect(socket_path).await?)
}

/// Send a request and await a JSON-RPC response on a simple (non-persistent) stream.
///
/// Skips notifications (which lack `id`) and returns the first response with a matching id.
/// For bidirectional communication with notifications, use `DaemonConnection`.
pub async fn send_request(stream: &mut UnixStream, request: &Request) -> Result<Response> {
    let (reader, writer) = stream.split();
    let mut writer = BufWriter::new(writer);
    let mut reader = BufReader::new(reader);

    let mut json = serde_json::to_vec(request)?;
    json.push(b'\n');
    writer.write_all(&json).await?;
    writer.flush().await?;

    let mut line = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line).await?;

        if line.trim().is_empty() {
            return Err(crate::ArshyError::Ipc("empty response from daemon".into()));
        }

        let val: serde_json::Value = serde_json::from_str(line.trim())?;

        // Skip notifications (have "method", no "id") — read until we get a response
        if val.get("method").is_some() {
            continue;
        }

        return Ok(serde_json::from_value::<Response>(val)?);
    }
}

/// Write a single JSON Line to a writer.
pub async fn write_json_line<W: AsyncWriteExt + Unpin, T: Serialize>(
    writer: &mut BufWriter<W>,
    value: &T,
) -> Result<()> {
    let mut json = serde_json::to_vec(value)?;
    json.push(b'\n');
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

// ── DaemonConnection — bidirectional proxy ↔ daemon ─────────────────────────

/// Pending response waiters: request id → oneshot sender.
type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

/// A persistent connection to the daemon supporting request/response.
///
/// Spawns two background tasks:
/// - **writer**: drains outgoing JSON to the socket
/// - **reader**: routes responses (by id) to pending waiters, notifications to a channel
///
/// Notifications are received via a separate `mpsc::Receiver<Notification>`
/// returned alongside the connection — this enables `tokio::select!` in the
/// proxy between stdin reads and notification forwarding.
pub struct DaemonConnection {
    write_tx: mpsc::Sender<serde_json::Value>,
    pending: PendingMap,
    closed: Arc<AtomicBool>,
    notification_drops: Arc<AtomicU64>,
    next_id: u64,
}

impl DaemonConnection {
    /// Wrap a `UnixStream` into a bidirectional connection.
    ///
    /// Returns the connection handle and a notification receiver channel.
    pub fn new(stream: UnixStream) -> (Self, mpsc::Receiver<Notification>) {
        let (reader_half, writer_half) = stream.into_split();
        let reader = BufReader::new(reader_half);
        let writer = BufWriter::new(writer_half);

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let notification_drops = Arc::new(AtomicU64::new(0));
        let (write_tx, write_rx) = mpsc::channel::<serde_json::Value>(64);
        let (notif_tx, notif_rx) = mpsc::channel::<Notification>(256);

        // Writer task
        let mut write_rx = write_rx;
        let writer_closed = closed.clone();
        tokio::spawn(async move {
            Self::writer_task(writer, &mut write_rx, writer_closed).await;
        });

        // Reader task
        let pending_clone = pending.clone();
        let reader_closed = closed.clone();
        let reader_notification_drops = notification_drops.clone();
        tokio::spawn(async move {
            Self::reader_task(
                reader,
                pending_clone,
                notif_tx,
                reader_closed,
                reader_notification_drops,
            )
            .await;
        });

        (Self { write_tx, pending, closed, notification_drops, next_id: 1 }, notif_rx)
    }

    /// Return and reset the number of live notifications dropped because the
    /// consumer was temporarily unable to keep up. Structured events remain
    /// available through `task/query`.
    pub fn take_notification_drops(&self) -> u64 {
        self.notification_drops.swap(0, Ordering::AcqRel)
    }

    pub fn notification_drops_handle(&self) -> Arc<AtomicU64> {
        self.notification_drops.clone()
    }

    /// Send a JSON-RPC request and wait for the response.
    ///
    /// Times out after 60 seconds if no response arrives.
    pub async fn send_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Response> {
        self.send_request_with_timeout(method, params, std::time::Duration::from_secs(60)).await
    }

    /// Send a JSON-RPC request with a custom timeout.
    pub async fn send_request_with_timeout(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout: std::time::Duration,
    ) -> Result<Response> {
        if self.closed.load(Ordering::Acquire) {
            return Err(crate::ArshyError::Ipc("daemon connection closed".into()));
        }

        let id = self.next_id;
        self.next_id += 1;

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        self.write_tx
            .send(request)
            .await
            .map_err(|_| crate::ArshyError::Ipc("daemon connection closed".into()))?;

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err(crate::ArshyError::Ipc("response channel dropped".into())),
            Err(_) => {
                // Clean up the pending entry on timeout
                self.pending.lock().await.remove(&id);
                Err(crate::ArshyError::Ipc(format!(
                    "request '{}' timed out after {}s",
                    method,
                    timeout.as_secs()
                )))
            }
        }
    }

    // ── Background tasks ────────────────────────────────────────────────────

    async fn writer_task(
        mut writer: BufWriter<tokio::net::unix::OwnedWriteHalf>,
        rx: &mut mpsc::Receiver<serde_json::Value>,
        closed: Arc<AtomicBool>,
    ) {
        while let Some(json) = rx.recv().await {
            match serde_json::to_vec(&json) {
                Ok(mut bytes) => {
                    bytes.push(b'\n');
                    if writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                    let _ = writer.flush().await;
                }
                Err(_) => continue,
            }
        }
        closed.store(true, Ordering::Release);
    }

    async fn reader_task(
        mut reader: BufReader<tokio::net::unix::OwnedReadHalf>,
        pending: PendingMap,
        notif_tx: mpsc::Sender<Notification>,
        closed: Arc<AtomicBool>,
        notification_drops: Arc<AtomicU64>,
    ) {
        let mut line = String::new();
        loop {
            line.clear();
            let n = match reader.read_line(&mut line).await {
                Ok(n) => n,
                Err(_) => break,
            };
            if n == 0 {
                break;
            }
            if line.trim().is_empty() {
                continue;
            }

            let val: serde_json::Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Route: response (has "id", no "method") vs notification (has "method")
            let has_id = val.get("id").is_some() && val["id"] != serde_json::Value::Null;
            let has_method = val.get("method").is_some();

            if has_id && !has_method {
                // Response — deliver to pending waiter
                if let Ok(resp) = serde_json::from_value::<Response>(val) {
                    let mut map = pending.lock().await;
                    if let Some(tx) = map.remove(&resp.id) {
                        let _ = tx.send(resp);
                    }
                }
            } else if has_method {
                // Notification — deliver to notification channel.
                if let Ok(notif) = serde_json::from_value::<Notification>(val) {
                    // Never block response parsing on live notification delivery.
                    // Structured events are persisted by the daemon and remain
                    // queryable even when a burst exceeds the live queue.
                    match notif_tx.try_send(notif) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            notification_drops.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                    }
                }
            }
        }
        closed.store(true, Ordering::Release);
        pending.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Request;

    /// Helper: create a connected pair of UnixStreams.
    fn pair() -> (UnixStream, UnixStream) {
        UnixStream::pair().expect("UnixStream::pair")
    }

    /// Helper: write a JSON line to a stream half.
    async fn write_line(stream: &mut tokio::net::unix::OwnedWriteHalf, val: &serde_json::Value) {
        let mut bytes = serde_json::to_vec(val).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).await.unwrap();
        stream.flush().await.unwrap();
    }

    #[tokio::test]
    async fn test_send_request_basic() {
        let (client, server) = pair();

        let handle = tokio::spawn(async move {
            let (mut sr, mut sw) = server.into_split();
            let mut reader = BufReader::new(&mut sr);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();

            let req: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(req["method"], "test/method");

            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": req["id"],
                "result": {"value": 42}
            });
            write_line(&mut sw, &resp).await;
        });

        let mut client_stream = client;
        let request = Request {
            jsonrpc: "2.0".into(),
            id: 1,
            method: "test/method".into(),
            params: serde_json::json!({}),
        };

        let response = send_request(&mut client_stream, &request).await.unwrap();
        assert_eq!(response.id, 1);
        assert_eq!(response.result["value"], 42);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_send_request_empty_response() {
        let (client, server) = pair();
        drop(server);

        let mut client_stream = client;
        let request = Request {
            jsonrpc: "2.0".into(),
            id: 1,
            method: "test".into(),
            params: serde_json::json!({}),
        };

        let result = send_request(&mut client_stream, &request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_daemon_connection_request_response() {
        let (client, server) = pair();
        let (mut conn, _notif_rx) = DaemonConnection::new(client);

        let handle = tokio::spawn(async move {
            let (mut sr, mut sw) = server.into_split();
            let mut reader = BufReader::new(&mut sr);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();

            let req: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(req["method"], "task/run");

            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": req["id"],
                "result": {"task_id": "abc-123", "status": "running"}
            });
            write_line(&mut sw, &resp).await;
        });

        let response =
            conn.send_request("task/run", serde_json::json!({"command": "echo hi"})).await.unwrap();
        assert_eq!(response.result["task_id"], "abc-123");
        assert_eq!(response.result["status"], "running");
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_daemon_connection_timeout() {
        let (client, server) = pair();
        let (mut conn, _notif_rx) = DaemonConnection::new(client);

        // Server reads but never responds; drops after test
        let handle = tokio::spawn(async move {
            let (mut sr, _sw) = server.into_split();
            let mut reader = BufReader::new(&mut sr);
            let mut line = String::new();
            let _ = reader.read_line(&mut line).await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        });

        let result = conn
            .send_request_with_timeout(
                "task/run",
                serde_json::json!({}),
                std::time::Duration::from_millis(100),
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("timed out"), "expected timeout error, got: {}", err);
        handle.abort();
    }

    #[tokio::test]
    async fn test_daemon_connection_peer_close_drops_pending_request() {
        let (client, server) = pair();
        let (mut conn, _notif_rx) = DaemonConnection::new(client);

        let handle = tokio::spawn(async move {
            let (mut sr, _sw) = server.into_split();
            let mut reader = BufReader::new(&mut sr);
            let mut line = String::new();
            let _ = reader.read_line(&mut line).await;
            // Simulate arshyd exiting after accepting the request without
            // returning a response.
        });

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            conn.send_request_with_timeout(
                "task/run",
                serde_json::json!({}),
                std::time::Duration::from_secs(2),
            ),
        )
        .await
        .expect("peer close should wake the pending request");

        assert!(result.is_err());
        assert!(
            !result.unwrap_err().to_string().contains("timed out"),
            "peer close must not wait for the request timeout"
        );
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_daemon_connection_notification_burst_does_not_block_response() {
        let (client, server) = pair();
        let (mut conn, _notif_rx) = DaemonConnection::new(client);

        let handle = tokio::spawn(async move {
            let (mut sr, mut sw) = server.into_split();
            let mut reader = BufReader::new(&mut sr);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let request: serde_json::Value = serde_json::from_str(line.trim()).unwrap();

            for seq in 0..300 {
                write_line(
                    &mut sw,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "diagnostic",
                        "params": {"seq": seq}
                    }),
                )
                .await;
            }
            write_line(
                &mut sw,
                &serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": {"ok": true}
                }),
            )
            .await;
        });

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            conn.send_request_with_timeout(
                "task/run",
                serde_json::json!({}),
                std::time::Duration::from_secs(5),
            ),
        )
        .await
        .expect("notification burst must not delay the response")
        .unwrap();

        assert_eq!(response.result["ok"], true);
        assert!(conn.take_notification_drops() > 0);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_daemon_connection_notification() {
        let (client, server) = pair();
        let (_conn, mut notif_rx) = DaemonConnection::new(client);

        let handle = tokio::spawn(async move {
            let (_sr, mut sw) = server.into_split();
            let notif = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "task/update",
                "params": {"task_id": "abc", "status": "running"}
            });
            write_line(&mut sw, &notif).await;
        });

        let notif = tokio::time::timeout(std::time::Duration::from_secs(2), notif_rx.recv()).await;
        assert!(notif.is_ok());
        let notif = notif.unwrap().unwrap();
        assert_eq!(notif.method, "task/update");
        assert_eq!(notif.params["task_id"], "abc");
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_daemon_connection_drain_notifications() {
        let (client, server) = pair();
        let (_conn, mut notif_rx) = DaemonConnection::new(client);

        let handle = tokio::spawn(async move {
            let (_sr, mut sw) = server.into_split();
            for i in 0..3 {
                let notif = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "diagnostic",
                    "params": {"seq": i}
                });
                write_line(&mut sw, &notif).await;
            }
        });

        // Wait for all notifications to arrive
        handle.await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut notifs = Vec::new();
        while let Ok(n) = notif_rx.try_recv() {
            notifs.push(n);
        }
        assert_eq!(notifs.len(), 3);
        assert_eq!(notifs[0].params["seq"], 0);
        assert_eq!(notifs[1].params["seq"], 1);
        assert_eq!(notifs[2].params["seq"], 2);
    }

    #[tokio::test]
    async fn test_daemon_connection_routes_response_and_notification() {
        let (client, server) = pair();
        let (mut conn, mut notif_rx) = DaemonConnection::new(client);

        let handle = tokio::spawn(async move {
            let (mut sr, mut sw) = server.into_split();
            let mut reader = BufReader::new(&mut sr);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();

            let req: serde_json::Value = serde_json::from_str(line.trim()).unwrap();

            // Send response
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": req["id"],
                "result": {"ok": true}
            });
            write_line(&mut sw, &resp).await;

            // Then send notification
            let notif = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "task/complete",
                "params": {"task_id": "abc", "exit_code": 0}
            });
            write_line(&mut sw, &notif).await;
        });

        // Send request — should get response
        let resp = conn.send_request("test", serde_json::json!({})).await.unwrap();
        assert_eq!(resp.result["ok"], true);

        // Then drain notifications — should get the notification
        handle.await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut notifs = Vec::new();
        while let Ok(n) = notif_rx.try_recv() {
            notifs.push(n);
        }
        assert_eq!(notifs.len(), 1);
        assert_eq!(notifs[0].method, "task/complete");
    }

    #[tokio::test]
    async fn test_daemon_connection_try_recv_empty() {
        let (client, server) = pair();
        let (_conn, mut notif_rx) = DaemonConnection::new(client);
        drop(server);

        let notif = notif_rx.try_recv().ok();
        assert!(notif.is_none());
    }

    #[tokio::test]
    async fn test_daemon_connection_closed_write() {
        let (client, server) = pair();
        let (mut conn, _notif_rx) = DaemonConnection::new(client);
        drop(server);

        let result = conn
            .send_request_with_timeout(
                "test",
                serde_json::json!({}),
                std::time::Duration::from_millis(500),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_json_line() {
        let (client, server) = pair();
        let mut writer = BufWriter::new(client);

        let value = serde_json::json!({"key": "value"});
        write_json_line(&mut writer, &value).await.unwrap();

        // Read from the other end of the pair
        let (mut sr, _sw) = server.into_split();
        let mut reader = BufReader::new(&mut sr);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["key"], "value");
    }
}
