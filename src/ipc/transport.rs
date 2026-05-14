//! UDS transport — connect, send JSON-RPC requests, read responses.

use super::{Notification, Request, Response};
use crate::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Connect to the daemon at the given UDS path.
pub async fn connect(socket_path: &std::path::Path) -> Result<UnixStream> {
    Ok(UnixStream::connect(socket_path).await?)
}

/// Send a request and await a JSON-RPC response on a simple (non-persistent) stream.
///
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
    reader.read_line(&mut line).await?;

    if line.trim().is_empty() {
        return Err(crate::ArshyError::Ipc("empty response from daemon".into()));
    }

    Ok(serde_json::from_str::<Response>(line.trim())?)
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

/// A persistent connection to the daemon supporting request/response and
/// receiving asynchronous notifications.
///
/// Spawns two background tasks:
/// - **writer**: drains outgoing JSON to the socket
/// - **reader**: routes responses (by id) to pending waiters, notifications to a channel
pub struct DaemonConnection {
    write_tx: mpsc::Sender<serde_json::Value>,
    notif_rx: mpsc::Receiver<Notification>,
    pending: PendingMap,
    next_id: u64,
}

impl DaemonConnection {
    /// Wrap a `UnixStream` into a bidirectional connection.
    pub fn new(stream: UnixStream) -> Self {
        let (reader_half, writer_half) = stream.into_split();
        let reader = BufReader::new(reader_half);
        let writer = BufWriter::new(writer_half);

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (write_tx, write_rx) = mpsc::channel::<serde_json::Value>(64);
        let (notif_tx, notif_rx) = mpsc::channel::<Notification>(256);

        // Writer task
        let mut write_rx = write_rx;
        tokio::spawn(async move {
            Self::writer_task(writer, &mut write_rx).await;
        });

        // Reader task
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            Self::reader_task(reader, pending_clone, notif_tx).await;
        });

        Self { write_tx, notif_rx, pending, next_id: 1 }
    }

    /// Send a JSON-RPC request and wait for the response.
    ///
    /// Times out after 60 seconds if no response arrives.
    pub async fn send_request(&mut self, method: &str, params: serde_json::Value) -> Result<Response> {
        self.send_request_with_timeout(method, params, std::time::Duration::from_secs(60)).await
    }

    /// Send a JSON-RPC request with a custom timeout.
    pub async fn send_request_with_timeout(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout: std::time::Duration,
    ) -> Result<Response> {
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

        self.write_tx.send(request).await
            .map_err(|_| crate::ArshyError::Ipc("daemon connection closed".into()))?;

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err(crate::ArshyError::Ipc("response channel dropped".into())),
            Err(_) => {
                // Clean up the pending entry on timeout
                self.pending.lock().await.remove(&id);
                Err(crate::ArshyError::Ipc(format!(
                    "request '{}' timed out after {}s", method, timeout.as_secs()
                )))
            }
        }
    }

    /// Try to receive a notification without blocking.
    pub fn try_recv_notification(&mut self) -> Option<Notification> {
        self.notif_rx.try_recv().ok()
    }

    /// Receive the next notification, waiting up to `timeout`.
    pub async fn recv_notification_timeout(&mut self, timeout: std::time::Duration) -> Option<Notification> {
        tokio::time::timeout(timeout, self.notif_rx.recv()).await.ok().flatten()
    }

    /// Drain all pending notifications (non-blocking).
    pub fn drain_notifications(&mut self) -> Vec<Notification> {
        let mut notifs = Vec::new();
        while let Ok(n) = self.notif_rx.try_recv() {
            notifs.push(n);
        }
        notifs
    }

    // ── Background tasks ────────────────────────────────────────────────────

    async fn writer_task(
        mut writer: BufWriter<tokio::net::unix::OwnedWriteHalf>,
        rx: &mut mpsc::Receiver<serde_json::Value>,
    ) {
        while let Some(json) = rx.recv().await {
            match serde_json::to_vec(&json) {
                Ok(mut bytes) => {
                    bytes.push(b'\n');
                    if writer.write_all(&bytes).await.is_err() { break; }
                    let _ = writer.flush().await;
                }
                Err(_) => continue,
            }
        }
    }

    async fn reader_task(
        mut reader: BufReader<tokio::net::unix::OwnedReadHalf>,
        pending: PendingMap,
        notif_tx: mpsc::Sender<Notification>,
    ) {
        let mut line = String::new();
        loop {
            line.clear();
            let n = match reader.read_line(&mut line).await {
                Ok(n) => n,
                Err(_) => break,
            };
            if n == 0 { break; }
            if line.trim().is_empty() { continue; }

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
                // Notification — deliver to notification channel
                if let Ok(notif) = serde_json::from_value::<Notification>(val) {
                    let _ = notif_tx.send(notif).await;
                }
            }
        }
    }
}
