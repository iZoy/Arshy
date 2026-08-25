//! Background task execution — long-running command lifecycle: PTY spawn,
//! six-layer parser pipeline, event storage, and completion handling.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::oneshot;

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
///
/// Note: error_count / warning_count are intentionally NOT here. The response
/// builds those counts from the same `events` array we ship to the agent
/// (see exec/mod.rs), so counting in the background task would create a
/// second, inconsistent source of truth (counting log events that the
/// response filters out).
pub(crate) struct CompletionInfo {
    pub(crate) status: TaskStatus,
    pub(crate) exit_code: i32,
    pub(crate) duration_ms: u64,
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

    let effective_timeout = t
        .timeout_ms
        .map(|value| value.min(t.config.max_task_duration_ms))
        .unwrap_or(t.config.max_task_duration_ms);
    let timeout_dur = tokio::time::Duration::from_millis(effective_timeout);

    let max_bytes = t.config.max_output_bytes;

    // 3-way select: output reading, timeout, or kill signal
    // After this select, handle.wait() is called to get the exit code.
    let mut captured_output = String::new();
    let mut seq = 0u64;
    // Keep parser state outside the select branch so timeout/kill cancellation
    // can still flush events already buffered by deduplication or pair/context
    // mergers. Dropping the branch used to silently lose the tail of noisy
    // commands when they timed out.
    let mut total_bytes: u64 = 0;
    let mut output_truncated = false;
    let mut saw_json_line = false;
    let mut dedup = Deduplicator::new();
    let mut ctx_merger = RustcContextMerger::new();
    let mut pair_merger = GenericPairMerger::new();
    let (timed_out, killed, dedup_collapsed, pairs_merged) = tokio::select! {
        result = async {
            while let Some((source, line)) = handle.output_rx.recv().await {
                let line_bytes = line.len() as u64 + 1;
                total_bytes = total_bytes.saturating_add(line_bytes);
                if output_truncated || total_bytes > max_bytes {
                    if output_truncated {
                        // Keep draining stdout/stderr so the child cannot
                        // block on a full pipe. Stopping the receiver at the
                        // capture limit deadlocks noisy processes in wait().
                        continue;
                    }
                    output_truncated = true;
                    tracing::warn!("task {} output exceeded {} bytes, truncating", t.task_id, max_bytes);
                    captured_output.push_str(&format!(
                        "[output truncated at {} bytes; remaining output drained]\n",
                        max_bytes
                    ));
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
                    continue;
                }
                captured_output.push_str(&line);
                captured_output.push('\n');

                // A valid object line is already parsed by ParserSession's
                // streaming JSON layer. Remember that fact so the completion
                // pass does not emit the same object a second time.
                if line.trim_start().starts_with('{')
                    && serde_json::from_str::<serde_json::Value>(line.trim()).is_ok()
                {
                    saw_json_line = true;
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

                for mut event in events {
                    // Normalize the source-sensitive severity before any
                    // buffering. Dedup/context/pair mergers may hold an event
                    // until a later line; applying this after those mergers
                    // loses the original stderr line and silently downgrades
                    // single-line failures such as "Permission denied".
                    if source == "stderr"
                        && matches!(event.severity.as_deref(), Some("info" | "warning"))
                        && stderr_looks_like_error(&event.message)
                    {
                        event.severity = Some("error".into());
                    }

                    if let Some(deduped) = dedup.feed(event) {
                        // Pair first so the location is attached to the
                        // diagnostic before rustc source context is buffered.
                        // The old order attached context to a location event,
                        // then discarded it when the pair merger copied only
                        // the location field.
                        for paired in pair_merger.feed_all(deduped) {
                            for mut event in ctx_merger.feed_all(paired) {
                                seq += 1;
                                event.seq = seq;

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
            // Output channel closed — process exited, readers finished. The
            // shared pipeline is flushed below, outside the select, so the
            // timeout and kill branches use the same finalization path.
            (dedup.collapsed_count(), pair_merger.merged_count())
        } => {
            (false, false, result.0, result.1)
        }
        _ = tokio::time::sleep(timeout_dur) => {
            tracing::warn!("task {} timed out after {}ms", t.task_id, timeout_dur.as_millis());
            let _ = handle.force_kill();
            let _ = t.store.update_task(&t.task_id, &TaskStatus::Timeout, Some(-2), None);
            (true, false, 0u64, 0u64)
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
            (false, true, 0u64, 0u64)
        }
    };

    // Flush the shared parser pipeline for normal completion, timeout, and
    // explicit kill alike. This is deliberately source-ordered and preserves
    // every event emitted by a merger when a buffered pair is terminated.
    let mut emit = |mut event: crate::ipc::TaskEvent| {
        seq += 1;
        event.seq = seq;
        if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
            tracing::error!("task {} failed to store final event: {}", t.task_id, e);
        }
        t.event_bus.publish(BusEvent {
            connection_id: 0,
            kind: BusEventKind::Diagnostic { task_id: t.task_id.clone(), event },
        });
    };
    if let Some(final_event) = dedup.finish() {
        for paired in pair_merger.feed_all(final_event) {
            for merged in ctx_merger.feed_all(paired) {
                emit(merged);
            }
        }
    }
    for final_event in pair_merger.finish_all() {
        for merged in ctx_merger.feed_all(final_event) {
            emit(merged);
        }
    }
    for final_event in ctx_merger.finish_all() {
        emit(final_event);
    }
    // Try JSON parsing on the full accumulated output only when streaming did
    // not already parse an object line (arrays and multi-line JSON still use
    // this completion pass).
    if !output_truncated && !saw_json_line {
        if let Some(json_events) = try_parse_json(&captured_output) {
            for mut event in json_events {
                seq += 1;
                event.seq = seq;
                if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
                    tracing::error!("task {} failed to store JSON event: {}", t.task_id, e);
                }
                t.event_bus.publish(BusEvent {
                    connection_id: 0,
                    kind: BusEventKind::Diagnostic { task_id: t.task_id.clone(), event },
                });
            }
        }
    }
    let raw_output = captured_output;

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

    let exit_code_val = exit_code.unwrap_or(if killed {
        -3
    } else if timed_out {
        -2
    } else {
        -1
    });

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

    // Enrichment belongs to task completion, not to any client connection.
    // The previous notification-side implementation meant detached async
    // tasks were never enriched after their proxy disconnected, while several
    // connected proxies could race to enrich the same task.
    let enrichment_params = crate::ipc::QueryParams {
        task_id: Some(t.task_id.clone()),
        event_type: None,
        severity: None,
        code: None,
        file: None,
        limit: 200,
        offset: 0,
        include_logs: false,
    };
    if let Ok((events, _)) = t.store.query_events(&enrichment_params) {
        if !events.is_empty() {
            let events_json: Vec<serde_json::Value> = events
                .into_iter()
                .map(|event| serde_json::to_value(event).unwrap_or_default())
                .collect();
            let cwd = t.cwd.clone().unwrap_or_else(|| std::path::PathBuf::from("."));
            let fallback = events_json.clone();
            let enriched =
                tokio::task::spawn_blocking(move || super::enrich_events(events_json, &cwd))
                    .await
                    .unwrap_or(fallback);
            let task_events: Vec<crate::ipc::TaskEvent> = enriched
                .into_iter()
                .filter_map(|event| serde_json::from_value(event).ok())
                .collect();
            if let Err(error) = t.store.merge_enriched_events(&t.task_id, &task_events) {
                tracing::warn!("failed to persist enriched events for {}: {}", t.task_id, error);
            }
        }
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
            status: final_status.as_str().into(),
            exit_code: exit_code_val,
            duration_ms,
        },
    });

    record_task_completed(final_status == TaskStatus::Completed);

    // Note: no error_count here — the response layer counts errors from the
    // actual events array it ships to the agent (see exec/mod.rs).
    tracing::info!(
        "task {} completed: status={:?}, exit_code={}, duration={}ms, events={}",
        t.task_id,
        final_status,
        exit_code_val,
        duration_ms,
        seq,
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
        let _ =
            tx.send(CompletionInfo { status: final_status, exit_code: exit_code_val, duration_ms });
    }

    Ok(())
}
