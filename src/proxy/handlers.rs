//! MCP request handlers — initialize, tools/list, tool calls, and resources.

use arshy_lib::ipc::{self, DaemonConnection};
use arshy_lib::mcp::{instructions, protocol};
use arshy_lib::Result;
use tokio::io::{AsyncWriteExt, BufWriter};

use super::protocol::{
    format_duration, negotiate_protocol_version, write_json_error, write_json_response,
};

// ── MCP handlers ────────────────────────────────────────────────────────────

pub(crate) async fn handle_initialize<W: tokio::io::AsyncWrite + Unpin, I: serde::Serialize>(
    stdout: &mut BufWriter<W>,
    id: I,
    request: &serde_json::Value,
) -> Result<()> {
    // Per MCP spec: respond with our supported version; the client decides
    // whether to proceed. Do NOT reject with an error on version mismatch.
    let protocol_version = negotiate_protocol_version(request);

    let caps = serde_json::json!({
        "protocolVersion": protocol_version,
        "serverInfo": {
            "name": "arshy",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "listChanged": false },
            "logging": {},
        },
        "instructions": instructions::default_instructions(),
    });
    write_json_response(stdout, id, &caps).await
}

pub(crate) async fn handle_tools_list<I: serde::Serialize>(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: I,
) -> Result<()> {
    let tools = instructions::tool_definitions();
    write_json_response(stdout, id, &serde_json::json!({ "tools": tools })).await
}

/// Shape an `arshy_query` daemon response for the MCP client: a concise
/// content line plus the structured `events` and `total` fields. Works for
/// both per-task queries and cross-task searches (events carry `task_id`).
pub(crate) fn build_query_result(result: &serde_json::Value) -> serde_json::Value {
    let events = result.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let total = result.get("total").and_then(|v| v.as_u64()).unwrap_or(events.len() as u64);
    let text = format!("{} event{}", total, if total == 1 { "" } else { "s" });
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "events": events,
        "total": total,
    })
}

/// Shape a `task/tail` daemon response for the MCP client: the content text
/// is the actual output lines (raw or event messages), and the structured
/// `lines` array + `task_id` are forwarded for programmatic access.
pub(crate) fn build_tail_result(result: &serde_json::Value) -> serde_json::Value {
    let lines = result.get("lines").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let text: String = lines.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("\n");
    let task_id = result.get("task_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "task_id": task_id,
        "lines": lines,
    })
}

fn build_task_list_result(result: &serde_json::Value) -> serde_json::Value {
    let tasks = result.as_array().cloned().unwrap_or_default();
    serde_json::json!({
        "content": [{"type": "text", "text": format!("{} task{}", tasks.len(), if tasks.len() == 1 { "" } else { "s" })}],
        "tasks": tasks,
    })
}

fn build_task_cancel_result(result: &serde_json::Value) -> serde_json::Value {
    let task_id = result.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    serde_json::json!({
        "content": [{"type": "text", "text": format!("cancellation requested for task {}", task_id)}],
        "task_id": task_id,
        "status": "cancelling",
    })
}

pub(crate) async fn handle_tool_call(
    daemon: &mut DaemonConnection,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    request: &serde_json::Value,
    id: &serde_json::Value,
    proxy_session_id: &str,
) -> Result<Option<String>> {
    let tool_name = request["params"]["name"].as_str().unwrap_or("");
    let mut args = request["params"]["arguments"].clone();
    let ipc_method = match mcp_tool_to_ipc_method(tool_name, &args) {
        Ok(method) => method,
        Err(e) => {
            write_json_error(stdout, id, e.json_rpc_code(), &e.to_string(), false).await?;
            return Ok(None);
        }
    };
    // `raw` is a first-class raw-output channel: force format=raw and default
    // to a generous line count (0 = all lines).
    if tool_name == "arshy_task" && args.get("action").and_then(|v| v.as_str()) == Some("raw") {
        if let Some(obj) = args.as_object_mut() {
            obj.insert("format".into(), serde_json::json!("raw"));
            obj.entry("lines").or_insert(serde_json::json!(200));
        }
    }
    // Replay protection: tag run requests with the MCP request id so a
    // replayed request (after proxy reconnect) is deduplicated by the daemon
    // instead of executing the command twice.
    if ipc_method == ipc::METHOD_RUN {
        if let Some(obj) = args.as_object_mut() {
            // The daemon is shared and may have been auto-started from a
            // different project. Preserve the MCP proxy's launch directory
            // when the client omits cwd; otherwise commands silently run in
            // the daemon's unrelated startup directory.
            if !obj.contains_key("cwd") {
                if let Ok(cwd) = std::env::current_dir() {
                    obj.insert("cwd".into(), serde_json::json!(cwd));
                }
            } else if let Some(relative) = obj.get("cwd").and_then(|value| value.as_str()) {
                let path = std::path::Path::new(relative);
                if path.is_relative() {
                    if let Ok(base) = std::env::current_dir() {
                        obj.insert("cwd".into(), serde_json::json!(base.join(path)));
                    }
                }
            }
            // MCP request ids are only unique within one client session. The
            // daemon cache is shared across proxy processes, so scope the key
            // by a per-proxy session nonce to prevent a new client reusing id
            // `2` from receiving another client's cached result.
            obj.insert(
                "dedup_key".into(),
                serde_json::json!(format!(
                    "{}:{}",
                    proxy_session_id,
                    serde_json::to_string(id).unwrap_or_else(|_| "null".into())
                )),
            );
        }
    }

    let response = daemon.send_request(ipc_method, args).await?;
    let result = &response.result;

    // Check for IPC-level error
    if result.get("error").is_some() {
        let err_msg = result["error"]["message"].as_str().unwrap_or("unknown error");
        let code =
            result["error"]["code"].as_i64().unwrap_or(arshy_lib::ipc::error_code::INTERNAL_ERROR);
        write_json_error(stdout, id, code, err_msg, false).await?;
        return Ok(None);
    }

    // arshy_query responses (per-task or cross-task search) are not run
    // results: build a concise content line and forward the daemon's
    // `events` + `total` as structured fields (total was previously dropped
    // by the generic run-response path).
    if ipc_method == ipc::METHOD_QUERY {
        write_json_response(stdout, id, &build_query_result(result)).await?;
        return Ok(None);
    }

    // task/tail (including action=raw) returns lines, not a run result:
    // the content text IS the output lines joined, with the structured
    // `lines` array + `task_id` forwarded for programmatic access.
    if ipc_method == ipc::METHOD_TAIL {
        write_json_response(stdout, id, &build_tail_result(result)).await?;
        return Ok(None);
    }

    if ipc_method == ipc::METHOD_LIST {
        write_json_response(stdout, id, &build_task_list_result(result)).await?;
        return Ok(None);
    }

    if ipc_method == ipc::METHOD_KILL {
        write_json_response(stdout, id, &build_task_cancel_result(result)).await?;
        return Ok(None);
    }

    // Extract task_id for cancellation tracking
    let task_id = result["task_id"].as_str().map(String::from);

    // Short command → plain text (like a native shell)
    let is_short = result["short_command"].as_bool().unwrap_or(false);
    let status = result["status"].as_str().unwrap_or("");
    let content = if is_short {
        let raw = result["raw_output"].as_str().unwrap_or("");
        // Provide meaningful message for timeouts
        let text = if raw.is_empty() && (status == "failed" || status == "timeout") {
            let label = if status == "timeout" { "timed out" } else { "failed" };
            format!("[command {}: exit code {}]", label, result["exit_code"].as_i64().unwrap_or(-1))
        } else {
            raw.to_string()
        };
        serde_json::json!([{"type": "text", "text": text}])
    } else {
        // Long command → concise structured summary.
        // Agent gets: status icon, duration, error count, root cause on failure.
        // Full events are available via arshy_query — not included here.
        let root_cause = result.get("root_cause");
        let error_count = result.get("error_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let exit_code = result.get("exit_code").and_then(|v| v.as_i64());
        let duration_ms = result.get("duration_ms").and_then(|v| v.as_u64());

        let status_icon = match status {
            "completed" => "✓",
            "failed" | "timeout" => "✗",
            "killed" => "⊘",
            "running" => "⟳",
            _ => "?",
        };

        let duration_str = format_duration(duration_ms);
        let has_errors = error_count > 0;
        let exit_nonzero = exit_code.is_some_and(|c| c != 0);

        // Build one-line summary: "✓ 0 errors, 2.3s" or "✗ 3 errors, 10.5s (exit 1)"
        let mut text = String::new();
        text.push_str(status_icon);
        text.push(' ');
        text.push_str(&format!("{} error{}", error_count, if error_count == 1 { "" } else { "s" }));
        if !duration_str.is_empty() {
            text.push_str(", ");
            text.push_str(&duration_str);
        }
        if exit_nonzero {
            if let Some(code) = exit_code {
                text.push_str(&format!(" (exit {})", code));
            }
        }

        // Attach root cause for failures (most useful single-line for the agent)
        if let Some(rc) = root_cause {
            if let Some(msg) = rc.get("message").and_then(|v| v.as_str()) {
                if !msg.is_empty() {
                    text.push_str(&format!("\nRoot cause: {}", msg));
                }
            }
        }

        // Include git diff stat on failure (helps agent correlate errors with changes)
        if has_errors || exit_nonzero {
            if let Some(pc) = result.get("project_context") {
                if let Some(stat) = pc.get("git_diff_stat").and_then(|v| v.as_str()) {
                    if !stat.is_empty() {
                        text.push_str(&format!("\nChanged files:\n{}", stat));
                    }
                }
            }
        }

        // Zero structured events → the agent cannot see what the command
        // printed. Point it at the raw-output channel instead of leaving it
        // blind (token restraint: hint only, no raw text inlined).
        let event_count = result.get("event_count").and_then(|v| v.as_u64()).unwrap_or(0);
        if event_count == 0 && status != "running" {
            if let Some(tid) = result.get("task_id").and_then(|v| v.as_str()) {
                if !tid.is_empty() {
                    text.push_str(&format!(
                        "\n(0 structured events — fetch the original output with \
                         arshy_task(action:\"raw\", task_id:\"{}\"))",
                        tid
                    ));
                }
            }
        }

        serde_json::json!([{"type": "text", "text": text}])
    };

    // Flag errors so agents can detect them programmatically via isError.
    // Only flag explicit failures and high exit codes (>=2).
    // exit_code=1 is ambiguous (grep no match, diff differs, test condition false)
    // and should not trigger MCP isError.
    let has_errors = result.get("error_count").and_then(|v| v.as_u64()).unwrap_or(0) > 0;
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64());
    let is_error = should_mark_tool_error(status, exit_code, has_errors);

    let mut result_obj = serde_json::json!({ "content": content });
    if is_error {
        result_obj["isError"] = serde_json::json!(true);
    }

    // Include full result metadata so agents (and scripts) can access structured data.
    // The MCP content[] has the human-readable summary; these fields give machines
    // programmatic access without a second round-trip to arshy_query.
    if !is_short {
        for key in &[
            "task_id",
            "status",
            "exit_code",
            "duration_ms",
            "error_count",
            "warning_count",
            "root_cause",
            "project_context",
        ] {
            if let Some(val) = result.get(*key) {
                result_obj[*key] = val.clone();
            }
        }
        // Include raw_output for agents that need the full command output
        if let Some(raw) = result.get("raw_output") {
            result_obj["raw_output"] = raw.clone();
        }
        // Forward events and event_count from the daemon's run response
        for key in &["events", "event_count"] {
            if let Some(val) = result.get(*key) {
                result_obj[*key] = val.clone();
            }
        }
    }

    let mcp_response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result_obj
    });

    let mut json = serde_json::to_vec(&mcp_response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(task_id)
}

fn should_mark_tool_error(status: &str, exit_code: Option<i64>, has_errors: bool) -> bool {
    let abnormal_exit = exit_code.is_some_and(|code| !(0..2).contains(&code));
    status == "timeout" || abnormal_exit || has_errors
}

// ── MCP resource handlers ───────────────────────────────────────────────────

pub(crate) async fn handle_resources_list(
    daemon: &mut DaemonConnection,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: &serde_json::Value,
) -> Result<()> {
    let response = daemon.send_request(ipc::METHOD_LIST, serde_json::json!({"limit": 50})).await?;
    if let Some(error) = response.result.get("error") {
        let code = error["code"].as_i64().unwrap_or(ipc::error_code::INTERNAL_ERROR);
        let message = error["message"].as_str().unwrap_or("daemon error");
        write_json_error(stdout, id, code, message, false).await?;
        return Ok(());
    }
    let tasks = response.result.as_array().cloned().unwrap_or_default();

    let resources: Vec<protocol::ResourceDefinition> = tasks
        .iter()
        .map(|t| {
            let task_id = t["task_id"].as_str().unwrap_or("");
            let command = t["command"].as_str().unwrap_or("");
            let status = t["status"].as_str().unwrap_or("unknown");
            protocol::ResourceDefinition {
                uri: format!("arshy://task/{}", task_id),
                name: format!("{} [{}]", command, status),
                description: Some(format!("Task {} — {}", task_id, command)),
                mime_type: Some("application/json".into()),
            }
        })
        .collect();

    write_json_response(stdout, id, &serde_json::json!({ "resources": resources })).await
}

pub(crate) async fn handle_resources_read(
    daemon: &mut DaemonConnection,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    request: &serde_json::Value,
    id: &serde_json::Value,
) -> Result<()> {
    let uri = request["params"]["uri"].as_str().unwrap_or("");
    let task_id = uri.strip_prefix("arshy://task/").unwrap_or("");

    if task_id.is_empty() {
        write_json_error(
            stdout,
            id,
            ipc::error_code::INVALID_PARAMS,
            "invalid resource URI",
            false,
        )
        .await?;
        return Ok(());
    }

    // Get task details via list (with task_id filter not available, query directly)
    let query_resp = daemon
        .send_request(
            ipc::METHOD_QUERY,
            serde_json::json!({
                "task_id": task_id,
                "limit": 200,
            }),
        )
        .await?;

    if let Some(error) = query_resp.result.get("error") {
        let code = error["code"].as_i64().unwrap_or(ipc::error_code::INTERNAL_ERROR);
        let message = error["message"].as_str().unwrap_or("daemon error");
        write_json_error(stdout, id, code, message, false).await?;
        return Ok(());
    }

    let events = query_resp.result["events"].as_array().cloned().unwrap_or_default();
    let total = query_resp.result["total"].as_u64().unwrap_or(0);

    let content = serde_json::json!({
        "task_id": task_id,
        "total_events": total,
        "events": events,
    });

    let text = serde_json::to_string_pretty(&content).unwrap_or_default();

    let resource_content = protocol::ResourceContent {
        uri: uri.to_string(),
        mime_type: Some("application/json".into()),
        text,
    };

    write_json_response(
        stdout,
        id,
        &serde_json::json!({
            "contents": [resource_content],
        }),
    )
    .await
}

/// Map MCP tool name to IPC method.
///
/// Public MCP model:
/// - `arshy_exec` → execute one command
/// - `arshy_query` → search structured diagnostic events
/// - `arshy_task` → cancel/list/raw lifecycle operations
///
/// Unknown tools are an error — a typo'd tool name must never silently
/// fall back to running a command.
pub(crate) fn mcp_tool_to_ipc_method(
    tool_name: &str,
    args: &serde_json::Value,
) -> Result<&'static str> {
    match tool_name {
        "arshy_exec" => Ok(ipc::METHOD_RUN),
        "arshy_query" => Ok(ipc::METHOD_QUERY),
        "arshy_task" => match args.get("action").and_then(|v| v.as_str()) {
            Some("cancel") | Some("kill") => Ok(ipc::METHOD_KILL),
            Some("list") => Ok(ipc::METHOD_LIST),
            Some("raw") => Ok(ipc::METHOD_TAIL),
            Some(other) => {
                Err(arshy_lib::ArshyError::Ipc(format!("unknown arshy_task action: {}", other)))
            }
            None => Err(arshy_lib::ArshyError::Ipc("missing arshy_task action".into())),
        },
        _ => Err(arshy_lib::ArshyError::Ipc(format!("unknown tool: {}", tool_name))),
    }
}

#[cfg(test)]
mod tests {
    use super::should_mark_tool_error;

    #[test]
    fn exit_one_without_diagnostics_is_not_an_mcp_transport_error() {
        assert!(!should_mark_tool_error("failed", Some(1), false));
        assert!(should_mark_tool_error("failed", Some(1), true));
        assert!(should_mark_tool_error("failed", Some(2), false));
        assert!(should_mark_tool_error("failed", Some(-1), false));
        assert!(should_mark_tool_error("timeout", Some(1), false));
    }
}
