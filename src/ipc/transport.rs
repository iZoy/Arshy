//! UDS transport — connect, send JSON-RPC requests, read responses.

use super::{Request, Response};
use crate::Result;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;

/// Connect to the daemon at the given UDS path.
pub async fn connect(socket_path: &std::path::Path) -> Result<UnixStream> {
    Ok(UnixStream::connect(socket_path).await?)
}

/// Send a request and await a JSON-RPC response.
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

/// Write a single JSON Line to a writer (for daemon → proxy responses).
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
