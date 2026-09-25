//! MCP request handlers — initialize, tools/list, tool calls, and resources.

use arshy_lib::ipc::{self, DaemonConnection};
use arshy_lib::mcp::{instructions, protocol};
use arshy_lib::Result;
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufWriter};

use super::protocol::{negotiate_protocol_version, write_json_error, write_json_response};

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

fn tool_result(text: String, structured: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": structured,
    })
}

/// Shape an `arshy_query` response using the standard structuredContent field
/// while keeping the complete page visible to text-only MCP clients.
pub(crate) fn build_query_result(
    result: &serde_json::Value,
    args: &serde_json::Value,
) -> serde_json::Value {
    let events = result.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let total = result.get("total").and_then(|v| v.as_u64()).unwrap_or(events.len() as u64);
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20).min(1000);
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
    let structured = serde_json::json!({
        "events": events,
        "total": total,
        "limit": limit,
        "offset": offset,
    });
    tool_result(serde_json::to_string(&structured).unwrap_or_default(), structured)
}

/// Shape a `task/tail` daemon response for the MCP client: the content text
/// is the actual output lines (raw or event messages), and the structured
/// `lines` array + `task_id` are forwarded for programmatic access.
pub(crate) fn build_tail_result(result: &serde_json::Value) -> serde_json::Value {
    let lines = result.get("lines").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let text: String = lines.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("\n");
    let task_id = result.get("task_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let structured = serde_json::json!({
        "task_id": task_id,
        "lines": lines,
    });
    tool_result(text, structured)
}

fn build_task_list_result(result: &serde_json::Value) -> serde_json::Value {
    let tasks = result.as_array().cloned().unwrap_or_default();
    let structured = serde_json::json!({
        "tasks": tasks,
        "total": tasks.len(),
    });
    tool_result(serde_json::to_string(&structured).unwrap_or_default(), structured)
}

fn build_task_cancel_result(result: &serde_json::Value) -> serde_json::Value {
    let task_id = result.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    let structured = serde_json::json!({
        "task_id": task_id,
        "status": "cancelling",
    });
    tool_result(format!("cancellation requested for task {task_id}"), structured)
}

pub(crate) async fn handle_tool_call(
    daemon: &mut DaemonConnection,
    pump: &mut super::NotificationPump<'_>,
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

    let response = pump
        .send_request(
            daemon,
            stdout,
            ipc_method,
            args.clone(),
            super::daemon_request_timeout(ipc_method, &args),
        )
        .await?;
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
        write_json_response(stdout, id, &build_query_result(result, &args)).await?;
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

    let (result_obj, task_id) = build_run_result(result);

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

fn build_run_result(result: &serde_json::Value) -> (serde_json::Value, Option<String>) {
    let status = result["status"].as_str().unwrap_or("");
    let exit_value = result.get("exit_code").cloned().unwrap_or(serde_json::Value::Null);
    let is_failure = status == "failed"
        || status == "timeout"
        || exit_value.as_i64().is_some_and(|code| code != 0);
    let raw = result.get("raw_output").and_then(|v| v.as_str()).unwrap_or("");
    let events = result.get("events").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let primary = result.get("primary_diagnostic").filter(|v| v.is_object());
    let task_id = result.get("task_id").and_then(|v| v.as_str()).map(str::to_owned);

    let (inline_raw, inline_truncated, omitted_bytes) = clip_inline_output(raw);
    let mut text = if !raw.is_empty() {
        inline_raw
    } else if is_failure {
        let messages = events
            .iter()
            .filter_map(|event| event.get("message").and_then(|v| v.as_str()))
            .filter(|message| !message.is_empty())
            .collect::<Vec<_>>();
        if messages.is_empty() {
            format!(
                "[command failed: exit code {}]",
                exit_value.as_i64().map_or_else(|| "unknown".into(), |code| code.to_string())
            )
        } else {
            messages.join("\n")
        }
    } else if status == "running" {
        String::new()
    } else {
        "✓".into()
    };
    if inline_truncated {
        text.push_str(&format!("\n[output clipped: {omitted_bytes} bytes omitted]"));
    }

    let raw_truncated = inline_truncated
        || raw.contains("[output truncated at ")
        || raw.contains("[line truncated at ")
        || (raw.is_empty()
            && result.get("raw_output_bytes").and_then(|v| v.as_u64()).unwrap_or(0) > 0);
    let event_count = result.get("event_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let events_not_fully_inlined = event_count > if is_failure { 1 } else { 0 };
    let events_incomplete =
        result.get("events_truncated").and_then(|v| v.as_bool()).unwrap_or(false)
            || events_not_fully_inlined;
    let needs_handle = status == "running" || events_incomplete || raw_truncated;
    let handle = if needs_handle { task_id.as_deref().filter(|tid| !tid.is_empty()) } else { None };
    if let Some(tid) = handle {
        let hint = if status == "running" {
            format!("\nTask {tid} is running; use arshy_task(action:\"cancel\", task_id:\"{tid}\") to stop it or arshy_query(task_id:\"{tid}\") to inspect results.")
        } else if raw_truncated {
            format!("\nOutput is incomplete; retrieve captured output with arshy_task(action:\"raw\", task_id:\"{tid}\", lines:0).")
        } else if events_incomplete {
            format!("\nMore diagnostics are available with arshy_query(task_id:\"{tid}\").")
        } else {
            format!(
                "\nMore output is available with arshy_task(action:\"raw\", task_id:\"{tid}\")."
            )
        };
        text.push_str(&hint);
    }
    let mut structured = serde_json::Map::new();
    structured.insert("status".into(), serde_json::json!(status));
    structured.insert("exit_code".into(), exit_value.clone());
    if is_failure {
        if let Some(diagnostic) = primary {
            let mut facts = serde_json::Map::new();
            for key in ["severity", "code", "location"] {
                if let Some(value) = diagnostic.get(key).filter(|value| !value.is_null()) {
                    facts.insert(key.into(), value.clone());
                }
            }
            if !facts.is_empty() {
                structured.insert("diagnostic".into(), serde_json::Value::Object(facts));
            }
        }
    }
    if let Some(tid) = handle {
        structured.insert("task_id".into(), serde_json::json!(tid));
    }
    let has_errors = result.get("error_count").and_then(|v| v.as_u64()).unwrap_or(0) > 0;
    let is_error = should_mark_tool_error(status, exit_value.as_i64(), has_errors);
    let mut result_obj = serde_json::json!({
        "content": [{"type":"text","text":text}],
        "structuredContent": serde_json::Value::Object(structured),
    });
    if is_error {
        result_obj["isError"] = serde_json::json!(true);
    }
    (result_obj, handle.map(str::to_owned))
}

fn clip_inline_output(raw: &str) -> (String, bool, usize) {
    let limit = arshy_lib::ipc::MCP_INLINE_OUTPUT_LIMIT_BYTES;
    if raw.len() <= limit {
        return (raw.to_owned(), false, 0);
    }
    let edge = limit / 2;
    let mut head_end = edge;
    while !raw.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = raw.len() - edge;
    while !raw.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let omitted = tail_start.saturating_sub(head_end);
    (format!("{}\n[…]\n{}", &raw[..head_end], &raw[tail_start..]), true, omitted)
}

fn should_mark_tool_error(status: &str, exit_code: Option<i64>, has_errors: bool) -> bool {
    let abnormal_exit = exit_code.is_some_and(|code| !(0..2).contains(&code));
    status == "timeout" || abnormal_exit || has_errors
}

// ── MCP resource handlers ───────────────────────────────────────────────────

pub(crate) async fn handle_resources_list(
    daemon: &mut DaemonConnection,
    pump: &mut super::NotificationPump<'_>,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: &serde_json::Value,
) -> Result<()> {
    let response = pump
        .send_request(
            daemon,
            stdout,
            ipc::METHOD_LIST,
            serde_json::json!({"limit": 50}),
            Duration::from_secs(60),
        )
        .await?;
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
    pump: &mut super::NotificationPump<'_>,
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
    let query_resp = pump
        .send_request(
            daemon,
            stdout,
            ipc::METHOD_QUERY,
            serde_json::json!({
                "task_id": task_id,
                "limit": 200,
            }),
            Duration::from_secs(60),
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
    use super::{build_query_result, build_run_result, clip_inline_output, should_mark_tool_error};

    fn text(result: &serde_json::Value) -> &str {
        result["content"][0]["text"].as_str().unwrap()
    }

    #[test]
    fn successful_inline_output_has_only_status_and_exit_code() {
        let input = serde_json::json!({
            "status":"completed", "exit_code":0, "task_id":"unused",
            "raw_output":"hello\n", "raw_output_bytes":6, "duration_ms":12, "event_count":0
        });
        let (result, handle) = build_run_result(&input);
        assert_eq!(text(&result), "hello\n");
        assert!(handle.is_none());
        assert_eq!(
            result["structuredContent"],
            serde_json::json!({"status":"completed","exit_code":0})
        );
    }

    #[test]
    fn empty_success_is_minimal() {
        let (result, _) =
            build_run_result(&serde_json::json!({"status":"completed","exit_code":0}));
        assert_eq!(text(&result), "✓");
    }

    #[test]
    fn failure_preserves_raw_error_and_structures_only_facts() {
        let input = serde_json::json!({
            "status":"failed", "exit_code":2, "raw_output":"error[E0308]: mismatched types\n",
            "error_count":1,
            "primary_diagnostic":{"message":"mismatched types","severity":"error","code":"E0308",
                "location":{"file":"src/main.rs","line":8},"context":{"line":"secret source"}}
        });
        let (result, _) = build_run_result(&input);
        assert_eq!(text(&result), "error[E0308]: mismatched types\n");
        assert_eq!(
            result["structuredContent"]["diagnostic"],
            serde_json::json!({
                "severity":"error", "code":"E0308", "location":{"file":"src/main.rs","line":8}
            })
        );
        assert!(result["structuredContent"]["diagnostic"].get("message").is_none());
        assert!(result["structuredContent"]["diagnostic"].get("context").is_none());
    }

    #[test]
    fn handle_is_exposed_only_for_running_or_incomplete_results() {
        let (running, running_handle) = build_run_result(&serde_json::json!({
            "status":"running","exit_code":null,"task_id":"task-1"
        }));
        assert_eq!(running_handle.as_deref(), Some("task-1"));
        assert_eq!(running["structuredContent"]["task_id"], "task-1");
        assert!(text(&running).contains("arshy_task(action:\"cancel\""));

        let (events, event_handle) = build_run_result(&serde_json::json!({
            "status":"completed","exit_code":0,"task_id":"task-2","events_truncated":true
        }));
        assert_eq!(event_handle.as_deref(), Some("task-2"));
        assert_eq!(events["structuredContent"]["task_id"], "task-2");
        assert!(text(&events).contains("arshy_query(task_id:\"task-2\")"));

        let (warning, warning_handle) = build_run_result(&serde_json::json!({
            "status":"completed","exit_code":0,"task_id":"task-warning","event_count":1
        }));
        assert_eq!(warning_handle.as_deref(), Some("task-warning"));
        assert!(text(&warning).contains("arshy_query(task_id:\"task-warning\")"));

        let (raw, raw_handle) = build_run_result(&serde_json::json!({
            "status":"completed","exit_code":0,"task_id":"task-3",
            "raw_output":"prefix\n[output truncated at 100 bytes]","raw_output_bytes":120
        }));
        assert_eq!(raw_handle.as_deref(), Some("task-3"));
        assert!(text(&raw).contains("arshy_task(action:\"raw\""));
    }

    #[test]
    fn long_output_is_clipped_at_utf8_boundaries_and_keeps_both_ends() {
        let raw =
            format!("start:{}終わり", "x".repeat(arshy_lib::ipc::MCP_INLINE_OUTPUT_LIMIT_BYTES));
        let (clipped, was_clipped, omitted) = clip_inline_output(&raw);
        assert!(was_clipped);
        assert!(omitted > 0);
        assert!(clipped.starts_with("start:"));
        assert!(clipped.ends_with("終わり"));

        let (response, task_id) = build_run_result(&serde_json::json!({
            "status":"completed", "exit_code":0, "task_id":"large-output", "raw_output":raw
        }));
        assert_eq!(task_id.as_deref(), Some("large-output"));
        assert!(text(&response).contains("output clipped"));
        assert!(text(&response).contains("lines:0"));
        assert_eq!(response["structuredContent"]["task_id"], "large-output");
    }

    #[test]
    fn exit_one_without_diagnostics_is_not_an_mcp_transport_error() {
        assert!(!should_mark_tool_error("failed", Some(1), false));
        assert!(should_mark_tool_error("failed", Some(1), true));
        assert!(should_mark_tool_error("failed", Some(2), false));
        assert!(should_mark_tool_error("failed", Some(-1), false));
        assert!(should_mark_tool_error("timeout", Some(1), false));
    }

    #[test]
    fn query_result_is_visible_to_structured_and_text_clients() {
        let result = serde_json::json!({
            "events": [{"type": "diagnostic", "message": "visible failure"}],
            "total": 3
        });
        let shaped = build_query_result(&result, &serde_json::json!({"limit": 1, "offset": 2}));
        assert_eq!(shaped["structuredContent"]["total"], 3);
        assert_eq!(shaped["structuredContent"]["limit"], 1);
        assert_eq!(shaped["structuredContent"]["offset"], 2);
        assert!(shaped["content"][0]["text"].as_str().unwrap().contains("visible failure"));
        assert!(shaped.get("events").is_none());
    }
}
