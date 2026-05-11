//! MCP stdio proxy — connects MCP clients (Claude Code, Cursor) to the arshy daemon.

use arshy_lib::config::Config;
use arshy_lib::mcp::{instructions, protocol};
use arshy_lib::ipc;
use arshy_lib::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

/// Run the MCP stdio proxy. Reads JSON-RPC from stdin, forwards tool calls to daemon, writes to stdout.
pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(proxy_main(config_path))
}

async fn proxy_main(config_path: Option<PathBuf>) -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides {
        config_path,
        ..Default::default()
    })?;

    let socket_path = cfg.daemon.expanded_socket_path();
    let mut daemon = connect_or_start(&cfg, &socket_path).await?;

    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = BufWriter::new(tokio::io::stdout());
    let mut line = String::new();

    loop {
        line.clear();
        let n = stdin.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }

        let request: serde_json::Value =
            serde_json::from_str(line.trim()).map_err(|e| arshy_lib::ArshyError::Mcp(e.to_string()))?;

        let method = request["method"].as_str().unwrap_or("");
        let id = request["id"].as_u64().unwrap_or(0);

        match method {
            "initialize" => {
                let mut notifications = HashMap::new();
                notifications.insert(
                    "diagnostic".to_string(),
                    serde_json::Value::Object(Default::default()),
                );

                let caps = protocol::ServerCapabilities {
                    protocol_version: "2024-11-05".into(),
                    server_info: protocol::ServerInfo {
                        name: "arshy".into(),
                        version: env!("CARGO_PKG_VERSION").into(),
                    },
                    capabilities: protocol::ServerFeatures {
                        tools: HashMap::new(),
                        notifications,
                    },
                    instructions: Some(instructions::default_instructions()),
                };
                write_json_response(&mut stdout, id, &serde_json::to_value(&caps)?).await?;
            }
            "tools/list" => {
                let tools = instructions::tool_definitions();
                write_json_response(&mut stdout, id, &serde_json::json!({ "tools": tools })).await?;
            }
            "tools/call" => {
                let tool_name = request["params"]["name"].as_str().unwrap_or("");
                let args = request["params"]["arguments"].clone();
                let response = forward_tool_call(&mut daemon, tool_name, args, id).await?;
                let mut json = serde_json::to_vec(&response)?;
                json.push(b'\n');
                stdout.write_all(&json).await?;
                stdout.flush().await?;
            }
            "notifications/initialized" => {
                // No response for notifications
            }
            _ => {
                write_json_error(
                    &mut stdout,
                    id,
                    -32601,
                    &format!("unknown method: {}", method),
                )
                .await?;
            }
        }
    }

    Ok(())
}

async fn connect_or_start(cfg: &Config, socket_path: &PathBuf) -> Result<tokio::net::UnixStream> {
    match ipc::connect(socket_path).await {
        Ok(s) => return Ok(s),
        Err(_) if cfg.daemon.auto_start => {
            start_daemon()?;
            for _ in 0..25 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if let Ok(s) = ipc::connect(socket_path).await {
                    return Ok(s);
                }
            }
            Err(arshy_lib::ArshyError::DaemonUnreachable(
                "daemon did not start within 5s".into(),
            ))
        }
        Err(e) => Err(e),
    }
}

fn start_daemon() -> Result<()> {
    std::process::Command::new("arshyd")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| arshy_lib::ArshyError::DaemonUnreachable(format!("spawn arshyd: {}", e)))?;
    Ok(())
}

async fn forward_tool_call(
    daemon: &mut tokio::net::UnixStream,
    tool_name: &str,
    args: serde_json::Value,
    id: u64,
) -> Result<serde_json::Value> {
    let ipc_method = mcp_tool_to_ipc_method(tool_name);

    let request = ipc::Request {
        jsonrpc: "2.0".into(),
        id,
        method: ipc_method.into(),
        params: args,
    };

    let response = ipc::send_request(daemon, &request).await?;

    let text = serde_json::to_string_pretty(&response.result).unwrap_or_default();
    Ok(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{"type": "text", "text": text}]
        }
    }))
}

fn mcp_tool_to_ipc_method(tool_name: &str) -> &str {
    match tool_name {
        "arshy_run" => ipc::METHOD_RUN,
        "arshy_query" => ipc::METHOD_QUERY,
        "arshy_list" => ipc::METHOD_LIST,
        "arshy_kill" => ipc::METHOD_KILL,
        "arshy_tail" => ipc::METHOD_TAIL,
        _ => ipc::METHOD_RUN,
    }
}

async fn write_json_response(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: u64,
    result: &serde_json::Value,
) -> Result<()> {
    let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let mut json = serde_json::to_vec(&response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}

async fn write_json_error(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: u64,
    code: i64,
    message: &str,
) -> Result<()> {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    });
    let mut json = serde_json::to_vec(&response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}
