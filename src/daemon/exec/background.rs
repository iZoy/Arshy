//! Background task execution — long-running command lifecycle: PTY spawn,
//! six-layer parser pipeline, event storage, and completion handling.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::oneshot;

use crate::daemon::context::extract_context_async;
use crate::daemon::context::git_correlator::GitCorrelation;
use crate::daemon::parser::dedup::Deduplicator;
use crate::daemon::parser::pair_merger::GenericPairMerger;
use crate::daemon::parser::stderr_looks_like_error;
use crate::daemon::parser::toml::raw_event;
use crate::daemon::parser::{try_parse_json, Engine, ParsedTool, RustcContextMerger};
use crate::daemon::security::{AuditEntry, AuditLog};
use crate::daemon::store::Store;
use crate::daemon::telemetry::record_task_completed;
use crate::ipc::TaskStatus;
use crate::Result;

use super::process;
use super::pty;
use super::ExecutorConfig;
use crate::daemon::bus::{BusEvent, BusEventKind, EventBus};

/// Completion info sent through the oneshot channel for sync mode.
pub(crate) struct CompletionInfo {
    pub(crate) status: TaskStatus,
    pub(crate) exit_code: i32,
    pub(crate) duration_ms: u64,
    pub(crate) error_count: u64,
    pub(crate) warning_count: u64,
}

/// Grouped parameters for a background task execution.
pub(crate) struct BackgroundTask {
    pub(crate) task_id: String,
    pub(crate) command: String,
    pub(crate) cwd: Option<std::path::PathBuf>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) detected_tool: Option<ParsedTool>,
    pub(crate) store: Arc<Store>,
    pub(crate) parser: Arc<Engine>,
    pub(crate) event_bus: EventBus,
    pub(crate) config: ExecutorConfig,
    pub(crate) audit_log: Option<Arc<AuditLog>>,
    pub(crate) done_tx: Option<oneshot::Sender<CompletionInfo>>,
    pub(crate) kill_rx: tokio::sync::mpsc::Receiver<()>,
    pub(crate) env: Option<HashMap<String, String>>,
}

/// Background task that runs the command, parses output, stores events.
pub(crate) async fn run_background(mut t: BackgroundTask) -> Result<()> {
    let start = std::time::Instant::now();

    // Create a parser session for this task (holds state for stateful parsers)
    let session = t.parser.create_session(t.detected_tool.as_ref());

    // Spawn the process
    let mut handle = pty::spawn_command(&t.command, t.cwd.as_deref(), t.env.as_ref()).await?;
    let pid = handle.pid; // captured for audit/debug, not exposed to agent

    // Update task with PID
    let _ = t.store.update_task_pid(&t.task_id, pid);

    // Publish task/update event
    t.event_bus.publish(BusEvent {
        connection_id: 0,
        kind: BusEventKind::TaskUpdate {
            task_id: t.task_id.clone(),
            status: "running".into(),
            elapsed_ms: 0,
        },
    });

    let timeout_dur =
        tokio::time::Duration::from_millis(t.timeout_ms.unwrap_or(t.config.max_task_duration_ms));

    let max_bytes = t.config.max_output_bytes;

    // 3-way select: output reading, timeout, or kill signal
    // After this select, handle.wait() is called to get the exit code.
    let (
        timed_out,
        killed,
        mut seq,
        error_count,
        warning_count,
        raw_output,
        dedup_collapsed,
        pairs_merged,
    ) = tokio::select! {
        result = async {
            let mut seq: u64 = 0;
            let mut total_bytes: u64 = 0;
            let mut error_count: u64 = 0;
            let mut warning_count: u64 = 0;
            let mut full_output = String::new();
            let mut dedup = Deduplicator::new();
            let mut ctx_merger = RustcContextMerger::new();
            let mut pair_merger = GenericPairMerger::new();
            while let Some((source, line)) = handle.output_rx.recv().await {
                full_output.push_str(&line);
                full_output.push('\n');
                total_bytes += line.len() as u64;
                if total_bytes > max_bytes {
                    tracing::warn!("task {} output exceeded {} bytes, truncating", t.task_id, max_bytes);
                    // Emit a system warning event about truncation
                    let truncation_event = crate::ipc::TaskEvent {
                        seq: 0,
                        event_type: "system".into(),
                        severity: Some("warning".into()),
                        code: None,
                        message: format!("Output truncated: exceeded {} byte limit", max_bytes),
                        location: None,
                        context: None,
                        hint: None,
                    };
                    seq += 1;
                    let mut te = truncation_event;
                    te.seq = seq;
                    if let Err(e) = t.store.insert_event(&t.task_id, seq, &te) {
                        tracing::error!("task {} failed to store truncation event: {}", t.task_id, e);
                    }
                    t.event_bus.publish(BusEvent {
                        connection_id: 0,
                        kind: BusEventKind::Diagnostic {
                            task_id: t.task_id.clone(),
                            event: te,
                        },
                    });
                    break;
                }

                // Catch parser panics to prevent one bad line from killing the task
                let events = {
                    let line_ref = &line;
                    let session_ref = &session;
                    let tool_ref = t.detected_tool.as_ref();
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        session_ref.parse_line(line_ref, seq, tool_ref)
                    })) {
                        Ok(events) => events,
                        Err(panic_info) => {
                            let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                                s.clone()
                            } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                                s.to_string()
                            } else {
                                "parser panic (no message)".to_string()
                            };
                            tracing::warn!("task {} parser panicked on line: {}", t.task_id, msg);
                            // Fallback to raw event
                            vec![raw_event(&line, seq)]
                        }
                    }
                };

                for event in events {
                    if let Some(deduped) = dedup.feed(event) {
                        // Feed through context merger: absorbs rustc context lines
                        // into the preceding diagnostic event
                        if let Some(ctx_merged) = ctx_merger.feed(deduped) {
                            // Feed through pair merger: absorbs diagnostic+location pairs
                            if let Some(mut event) = pair_merger.feed(ctx_merged) {
                                seq += 1;
                                event.seq = seq;

                                if source == "stderr" {
                                    match event.severity.as_deref() {
                                        Some("info") => {
                                            if stderr_looks_like_error(&line) {
                                                event.severity = Some("error".into());
                                            } else {
                                                event.severity = Some("warning".into());
                                            }
                                        }
                                        Some("warning")
                                            if stderr_looks_like_error(&line) =>
                                        {
                                            event.severity = Some("error".into());
                                        }
                                        _ => {}
                                    }
                                }

                                if event.severity.as_deref() == Some("error") {
                                    error_count += 1;
                                }
                                if event.severity.as_deref() == Some("warning") {
                                    warning_count += 1;
                                }

                                // Extract error context (source file +/- 3 lines) for events with location
                                if let Some(ref loc) = event.location {
                                    if loc.line > 0 && !loc.file.is_empty() {
                                        if let Some(ctx) =
                                            extract_context_async(&loc.file, loc.line).await
                                        {
                                            event.context = Some(ctx);
                                        }
                                    }
                                }

                                if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
                                    tracing::error!(
                                        "task {} failed to store event: {}",
                                        t.task_id,
                                        e
                                    );
                                }

                                t.event_bus.publish(BusEvent {
                                    connection_id: 0,
                                    kind: BusEventKind::Diagnostic {
                                        task_id: t.task_id.clone(),
                                        event,
                                    },
                                });
                            }
                        }
                    }
                }
            }
            // Flush remaining deduplicated and context-merged events
            // dedup → ctx_merger → pair_merger (full pipeline)
            if let Some(final_event) = dedup.finish() {
                if let Some(ctx_merged) = ctx_merger.feed(final_event) {
                    if let Some(merged_event) = pair_merger.feed(ctx_merged) {
                        seq += 1;
                        let mut event = merged_event;
                        event.seq = seq;
                        if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
                            tracing::error!(
                                "task {} failed to store dedup event: {}",
                                t.task_id,
                                e
                            );
                        }
                        t.event_bus.publish(BusEvent {
                            connection_id: 0,
                            kind: BusEventKind::Diagnostic {
                                task_id: t.task_id.clone(),
                                event,
                            },
                        });
                    }
                }
            }
            // Flush any remaining buffered event in the context merger
            // ctx_merger → pair_merger (partial pipeline)
            if let Some(final_event) = ctx_merger.finish() {
                if let Some(merged_event) = pair_merger.feed(final_event) {
                    seq += 1;
                    let mut event = merged_event;
                    event.seq = seq;
                    if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
                        tracing::error!(
                            "task {} failed to store merger event: {}",
                            t.task_id,
                            e
                        );
                    }
                    t.event_bus.publish(BusEvent {
                        connection_id: 0,
                        kind: BusEventKind::Diagnostic {
                            task_id: t.task_id.clone(),
                            event,
                        },
                    });
                }
            }
            // Flush any remaining buffered event in the pair merger
            if let Some(final_event) = pair_merger.finish() {
                seq += 1;
                let mut event = final_event;
                event.seq = seq;
                if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
                    tracing::error!(
                        "task {} failed to store pair merger event: {}",
                        t.task_id,
                        e
                    );
                }
                t.event_bus.publish(BusEvent {
                    connection_id: 0,
                    kind: BusEventKind::Diagnostic {
                        task_id: t.task_id.clone(),
                        event,
                    },
                });
            }
            // Try JSON parsing on the full accumulated output
            if let Some(json_events) = try_parse_json(&full_output) {
                for mut event in json_events {
                    seq += 1;
                    event.seq = seq;
                    if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
                        tracing::error!("task {} failed to store JSON event: {}", t.task_id, e);
                    }
                    t.event_bus.publish(BusEvent {
                        connection_id: 0,
                        kind: BusEventKind::Diagnostic {
                            task_id: t.task_id.clone(),
                            event,
                        },
                    });
                }
            }
            // Output channel closed — process exited, readers finished
            (seq, error_count, warning_count, full_output, dedup.collapsed_count(), pair_merger.merged_count())
        } => {
            (false, false, result.0, result.1, result.2, result.3, result.4, result.5)
        }
        _ = tokio::time::sleep(timeout_dur) => {
            tracing::warn!("task {} timed out after {}ms", t.task_id, timeout_dur.as_millis());
            let _ = handle.force_kill();
            let _ = t.store.update_task(&t.task_id, &TaskStatus::Timeout, Some(-2), None);
            (true, false, 0u64, 0u64, 0u64, String::new(), 0u64, 0u64)
        }
        _ = t.kill_rx.recv() => {
            tracing::info!("task {} received kill signal, initiating graceful kill", t.task_id);
            // Graceful kill: SIGINT → wait → SIGTERM → wait → SIGKILL
            let grace_ms = t.config.kill_graceful_ms;
            let force_ms = t.config.kill_force_ms;
            match process::graceful_kill(&mut handle, grace_ms, force_ms).await {
                Ok(true) => tracing::debug!("task {} exited gracefully", t.task_id),
                Ok(false) => tracing::warn!("task {} was force-killed", t.task_id),
                Err(e) => tracing::error!("task {} kill error: {}", t.task_id, e),
            }
            (false, true, 0u64, 0u64, 0u64, String::new(), 0u64, 0u64)
        }
    };

    // Wait for the process to exit (output readers are done, process should be done or dying)
    let exit_code = match handle.wait().await {
        Ok(code) => code,
        Err(e) => {
            tracing::error!("task {} wait error: {}", t.task_id, e);
            Some(-1)
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let final_status = if killed {
        TaskStatus::Killed
    } else if timed_out {
        TaskStatus::Timeout
    } else {
        match exit_code {
            Some(0) => TaskStatus::Completed,
            Some(_) => TaskStatus::Failed,
            None => TaskStatus::Timeout,
        }
    };

    let exit_code_val = exit_code.unwrap_or(if killed { -3 } else { -1 });

    // Store raw output for tee / failure recovery
    if !raw_output.is_empty() {
        if let Err(e) = t.store.update_task_raw_output(&t.task_id, &raw_output) {
            tracing::warn!("failed to store raw output for task {}: {}", t.task_id, e);
        }
        if let Err(e) = t.store.update_task_raw_output_bytes(&t.task_id, raw_output.len() as u64) {
            tracing::warn!("failed to store raw output bytes for task {}: {}", t.task_id, e);
        }
    }

    // Emit completion events from stateful parsers
    let completion_events = session.on_complete(exit_code_val, seq);
    for mut event in completion_events {
        seq += 1;
        event.seq = seq;
        if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
            tracing::error!("task {} failed to store completion event: {}", t.task_id, e);
        }
        t.event_bus.publish(BusEvent {
            connection_id: 0,
            kind: BusEventKind::Diagnostic { task_id: t.task_id.clone(), event },
        });
    }

    // Update task in DB
    if let Err(e) =
        t.store.update_task(&t.task_id, &final_status, Some(exit_code_val), Some(duration_ms))
    {
        tracing::error!("task {} failed to update final status: {}", t.task_id, e);
    }

    // Store feature usage counters
    if dedup_collapsed > 0 {
        if let Err(e) = t.store.update_task_counters(&t.task_id, dedup_collapsed, 0) {
            tracing::warn!("failed to store dedup counter for task {}: {}", t.task_id, e);
        }
    }
    if pairs_merged > 0 {
        if let Err(e) = t.store.update_task_pairs_merged(&t.task_id, pairs_merged) {
            tracing::warn!("failed to store pairs_merged counter for task {}: {}", t.task_id, e);
        }
    }

    // Git correlation: count errors linked to recently changed files.
    // This runs for all tasks (sync and async), so correlated_errors is always tracked.
    if let Some(ref cwd_path) = t.cwd {
        let store = t.store.clone();
        let task_id = t.task_id.clone();
        let cwd = cwd_path.clone();
        tokio::task::spawn_blocking(move || {
            let params = crate::ipc::QueryParams {
                task_id: Some(task_id.clone()),
                event_type: None,
                severity: Some("error".into()),
                code: None,
                file: None,
                limit: 200,
                offset: 0,
                include_logs: false,
            };
            if let Ok((events, _)) = store.query_events(&params) {
                if let Some(gc) = GitCorrelation::detect(Some(cwd.as_path())) {
                    let count = events
                        .iter()
                        .filter(|e| {
                            e.location.as_ref().is_some_and(|loc| {
                                gc.changed_files().iter().any(|f| f == &loc.file)
                            })
                        })
                        .count() as u64;
                    if count > 0 {
                        let _ = store.update_task_counters(&task_id, 0, count);
                    }
                }
            }
        });
    }

    // Refresh the idle watchdog's activity stamp at completion so "idle"
    // counts from the end of the task, not its start.
    t.store.mark_activity();

    // Publish completion event
    t.event_bus.publish(BusEvent {
        connection_id: 0,
        kind: BusEventKind::TaskComplete {
            task_id: t.task_id.clone(),
            exit_code: exit_code_val,
            duration_ms,
        },
    });

    record_task_completed(exit_code_val == 0);

    tracing::info!(
        "task {} completed: status={:?}, exit_code={}, duration={}ms, events={}, errors={}",
        t.task_id,
        final_status,
        exit_code_val,
        duration_ms,
        seq,
        error_count
    );

    // Audit log: task completed
    if let Some(ref audit) = t.audit_log {
        let _ = audit.log(&AuditEntry {
            timestamp: chrono::Utc::now(),
            task_id: t.task_id.clone(),
            command: t.command.clone(),
            cwd: t.cwd.as_ref().map(|p| p.to_string_lossy().to_string()),
            exit_code: Some(exit_code_val),
            blocked: false,
            reason: None,
        });
    }

    // Signal sync waiters
    if let Some(tx) = t.done_tx {
        let _ = tx.send(CompletionInfo {
            status: final_status,
            exit_code: exit_code_val,
            duration_ms,
            error_count,
            warning_count,
        });
    }

    Ok(())
}
