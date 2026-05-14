//! Execution engine — spawns commands, manages task lifecycle, streams events.

pub mod process;
pub mod pty;

use arshy_lib::ipc::{Task, TaskStatus};
use arshy_lib::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex as TokioMutex};

use super::bus::{BusEvent, BusEventKind, EventBus};
use super::context;
use super::ipc_handler::RunResult;
use super::parser::{Engine, ParsedTool};
use super::store::Store;

/// Core executor that owns the store, parser, and event bus.
pub struct Executor {
    store: Arc<Store>,
    parser: Arc<Engine>,
    event_bus: EventBus,
    config: ExecutorConfig,
    /// Registry of running tasks' kill signal senders.
    kill_registry: Arc<TokioMutex<HashMap<String, tokio::sync::mpsc::Sender<()>>>>,
}

/// Runtime configuration for task execution.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub max_task_duration_ms: u64,
    pub max_output_bytes: u64,
    pub kill_graceful_ms: u64,
    pub kill_force_ms: u64,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_task_duration_ms: 3_600_000, // 1 hour
            max_output_bytes: 10_485_760,     // 10 MB
            kill_graceful_ms: 3_000,
            kill_force_ms: 2_000,
        }
    }
}

impl Executor {
    pub fn new(store: Arc<Store>, parser: Arc<Engine>, event_bus: EventBus) -> Self {
        Self {
            store,
            parser,
            event_bus,
            config: ExecutorConfig::default(),
            kill_registry: Arc::new(TokioMutex::new(HashMap::new())),
        }
    }

    pub fn with_config(mut self, config: ExecutorConfig) -> Self {
        self.config = config;
        self
    }

    /// Schedule a command for execution.
    ///
    /// In sync mode, waits for completion and returns the full result.
    /// In async mode, returns immediately with the task_id.
    pub async fn run(
        &self,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        mode: &str,
    ) -> Result<RunResult> {
        let tool = self.parser.detect(command);
        let task_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let cwd_string = cwd.map(String::from);

        let task = Task {
            task_id: task_id.clone(),
            command: command.to_string(),
            cwd: cwd_string.clone(),
            status: TaskStatus::Running,
            exit_code: None,
            pid: None,
            parser_name: tool.as_ref().map(|t| t.parser_name.clone()),
            started_at: now,
            finished_at: None,
            duration_ms: None,
            events_count: 0,
            error_count: 0,
        };
        self.store.insert_task(&task)?;

        let is_sync = mode == "sync" || mode == "auto";

        // Spawn background execution task
        let store = self.store.clone();
        let parser = self.parser.clone();
        let event_bus = self.event_bus.clone();
        let executor_config = self.config.clone();
        let cmd = command.to_string();
        let task_id_bg = task_id.clone();
        let detected_tool = tool;

        // For sync mode, use a oneshot channel to wait for completion
        let (done_tx, done_rx) = if is_sync {
            let (tx, rx) = oneshot::channel::<CompletionInfo>();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        // Kill signal channel — executor sends, background task receives
        let (kill_tx, kill_rx) = tokio::sync::mpsc::channel::<()>(1);
        self.kill_registry.lock().await.insert(task_id.clone(), kill_tx);

        let kill_registry = self.kill_registry.clone();
        let task = BackgroundTask {
            task_id: task_id_bg,
            command: cmd,
            cwd: cwd_string.map(std::path::PathBuf::from),
            timeout_ms,
            detected_tool,
            store,
            parser,
            event_bus,
            config: executor_config,
            done_tx,
            kill_rx,
        };
        let task_id_cleanup = task_id.clone();
        tokio::spawn(async move {
            if let Err(e) = run_background(task).await {
                tracing::error!("background task failed: {}", e);
            }
            // Clean up kill registry entry
            kill_registry.lock().await.remove(&task_id_cleanup);
        });

        // Async mode: return immediately
        if !is_sync {
            return Ok(RunResult {
                task_id,
                status: TaskStatus::Running,
                pid: None,
                exit_code: None,
                duration_ms: None,
                event_count: None,
                error_count: None,
            });
        }

        // Sync mode: wait for completion
        match done_rx {
            Some(rx) => match rx.await {
                Ok(info) => Ok(RunResult {
                    task_id,
                    status: info.status,
                    pid: info.pid,
                    exit_code: Some(info.exit_code),
                    duration_ms: Some(info.duration_ms),
                    event_count: Some(info.event_count),
                    error_count: Some(info.error_count),
                }),
                Err(_) => {
                    // Channel dropped — task panicked or was cancelled
                    Ok(RunResult {
                        task_id,
                        status: TaskStatus::Failed,
                        pid: None,
                        exit_code: Some(-1),
                        duration_ms: None,
                        event_count: None,
                        error_count: None,
                    })
                }
            },
            None => unreachable!(),
        }
    }

    /// Kill a running task gracefully (SIGINT → SIGTERM → SIGKILL).
    pub async fn kill(&self, task_id: &str) -> Result<()> {
        let kill_tx = self.kill_registry.lock().await.remove(task_id);
        match kill_tx {
            Some(tx) => {
                // Signal the background task to initiate graceful kill
                let _ = tx.send(()).await;
                Ok(())
            }
            None => {
                // Task not found in registry — may have already finished.
                // Mark as killed in DB anyway.
                self.store.update_task(task_id, &TaskStatus::Killed, Some(-1), None)?;
                Ok(())
            }
        }
    }

    /// Tail the most recent events of a task.
    pub async fn tail(&self, task_id: &str, lines: usize, _format: &str) -> Result<Vec<String>> {
        use arshy_lib::ipc::QueryParams;
        let params = QueryParams {
            task_id: task_id.to_string(),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: lines,
            offset: 0,
        };
        let (events, _total) = self.store.query_events(&params)?;
        Ok(events.into_iter().map(|e| e.message).collect())
    }
}

/// Completion info sent through the oneshot channel for sync mode.
struct CompletionInfo {
    status: TaskStatus,
    pid: Option<u32>,
    exit_code: i32,
    duration_ms: u64,
    event_count: u64,
    error_count: u64,
}

/// Grouped parameters for a background task execution.
struct BackgroundTask {
    task_id: String,
    command: String,
    cwd: Option<std::path::PathBuf>,
    timeout_ms: Option<u64>,
    detected_tool: Option<ParsedTool>,
    store: Arc<Store>,
    parser: Arc<Engine>,
    event_bus: EventBus,
    config: ExecutorConfig,
    done_tx: Option<oneshot::Sender<CompletionInfo>>,
    kill_rx: tokio::sync::mpsc::Receiver<()>,
}

/// Background task that runs the command, parses output, stores events.
async fn run_background(mut t: BackgroundTask) -> Result<()> {
    let start = std::time::Instant::now();

    // Probe tool version (cached in SQLite, refreshed every 24h)
    if let Some(ref mut tool) = t.detected_tool {
        if let Some(version) = super::parser::probe_version(
            &tool.tool_name,
            &t.store,
            t.parser.config().version_cache_ttl_hours,
        ).await {
            tool.version = Some(version);
        }
    }

    // Create a parser session for this task (holds state for stateful parsers)
    let session = t.parser.create_session(t.detected_tool.as_ref());

    // Spawn the process
    let mut handle = pty::spawn_command(&t.command, t.cwd.as_deref()).await?;
    let pid = handle.pid;

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

    let timeout_dur = tokio::time::Duration::from_millis(
        t.timeout_ms.unwrap_or(t.config.max_task_duration_ms),
    );

    let max_bytes = t.config.max_output_bytes;

    // 3-way select: output reading, timeout, or kill signal
    // After this select, handle.wait() is called to get the exit code.
    let (timed_out, killed, mut seq, error_count) = tokio::select! {
        result = async {
            let mut seq: u64 = 0;
            let mut total_bytes: u64 = 0;
            let mut error_count: u64 = 0;
            while let Some((source, line)) = handle.output_rx.recv().await {
                total_bytes += line.len() as u64;
                if total_bytes > max_bytes {
                    tracing::warn!("task {} output exceeded {} bytes, truncating", t.task_id, max_bytes);
                    // Emit a system warning event about truncation
                    let truncation_event = arshy_lib::ipc::TaskEvent {
                        seq: 0,
                        event_type: "system".into(),
                        severity: Some("warning".into()),
                        code: None,
                        message: format!("Output truncated: exceeded {} byte limit", max_bytes),
                        location: None,
                        context: None,
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
                            vec![super::parser::toml::raw_event(&line, seq)]
                        }
                    }
                };

                for mut event in events {
                    seq += 1;
                    event.seq = seq;

                    if source == "stderr" && event.severity.as_deref() == Some("info") {
                        event.severity = Some("warning".into());
                    }

                    if event.severity.as_deref() == Some("error") {
                        error_count += 1;
                    }

                    // Extract error context (source file ±3 lines) for events with location
                    if let Some(ref loc) = event.location {
                        if loc.line > 0 && !loc.file.is_empty() {
                            if let Some(ctx) = context::extract_context_async(&loc.file, loc.line).await {
                                event.context = Some(ctx);
                            }
                        }
                    }

                    if let Err(e) = t.store.insert_event(&t.task_id, seq, &event) {
                        tracing::error!("task {} failed to store event: {}", t.task_id, e);
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
            (seq, error_count)
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
            kind: BusEventKind::Diagnostic {
                task_id: t.task_id.clone(),
                event,
            },
        });
    }

    // Update task in DB
    if let Err(e) = t.store.update_task(&t.task_id, &final_status, Some(exit_code_val), Some(duration_ms)) {
        tracing::error!("task {} failed to update final status: {}", t.task_id, e);
    }

    // Publish completion event
    t.event_bus.publish(BusEvent {
        connection_id: 0,
        kind: BusEventKind::TaskComplete {
            task_id: t.task_id.clone(),
            exit_code: exit_code_val,
            duration_ms,
        },
    });

    tracing::info!(
        "task {} completed: status={:?}, exit_code={}, duration={}ms, events={}, errors={}",
        t.task_id, final_status, exit_code_val, duration_ms, seq, error_count
    );

    // Signal sync waiters
    if let Some(tx) = t.done_tx {
        let _ = tx.send(CompletionInfo {
            status: final_status,
            pid: Some(pid),
            exit_code: exit_code_val,
            duration_ms,
            event_count: seq,
            error_count,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::config::ParserConfig;
    use tempfile::TempDir;

    fn setup() -> (Arc<Store>, Arc<Engine>, EventBus, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Arc::new(Store::open(&db_path, false).unwrap());
        store.initialize_schema().unwrap();
        let parser = Arc::new(Engine::new(&ParserConfig::default()).unwrap());
        let bus = EventBus::new();
        (store, parser, bus, tmp)
    }

    #[tokio::test]
    async fn test_executor_run_echo() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("echo hello", None, None, "async").await.unwrap();
        assert!(!result.task_id.is_empty());
        assert_eq!(result.status, TaskStatus::Running);

        // Wait for the background task to complete
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Check the task was updated in the store
        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.exit_code, Some(0));
        assert!(task.duration_ms.is_some());

        // Check events were stored
        use arshy_lib::ipc::QueryParams;
        let params = QueryParams {
            task_id: result.task_id.clone(),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert!(total >= 1, "expected at least 1 event, got {}", total);
        assert_eq!(events[0].message, "hello");
    }

    #[tokio::test]
    async fn test_executor_run_failure() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("exit 1", None, None, "async").await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.exit_code, Some(1));
    }

    #[tokio::test]
    async fn test_executor_tail() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("printf 'a\nb\nc\n'", None, None, "async").await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let lines = executor.tail(&result.task_id, 10, "raw").await.unwrap();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn test_executor_timeout() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus)
            .with_config(ExecutorConfig {
                max_task_duration_ms: 500,
                ..Default::default()
            });

        let result = executor.run("sleep 60", None, Some(500), "async").await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let task = store.get_task(&result.task_id).unwrap().unwrap();
        // Should be either Timeout or Killed (depending on timing)
        assert!(
            task.status == TaskStatus::Timeout || task.status == TaskStatus::Killed,
            "expected Timeout or Killed, got {:?}",
            task.status
        );
    }

    #[tokio::test]
    async fn test_executor_sync_mode() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("echo sync_test", None, None, "sync").await.unwrap();
        // Sync mode should wait for completion and return full result
        assert_eq!(result.status, TaskStatus::Completed);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.duration_ms.is_some());
        assert!(result.event_count.unwrap() >= 1);
    }

    #[tokio::test]
    async fn test_executor_sync_mode_failure() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("exit 42", None, None, "sync").await.unwrap();
        assert_eq!(result.status, TaskStatus::Failed);
        assert_eq!(result.exit_code, Some(42));
    }

    #[tokio::test]
    async fn test_executor_kill_graceful() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("sleep 60", None, None, "async").await.unwrap();
        assert_eq!(result.status, TaskStatus::Running);

        // Wait a bit for the process to start
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Kill it
        executor.kill(&result.task_id).await.unwrap();

        // Wait for kill to take effect
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Check task was updated
        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert!(
            task.status == TaskStatus::Killed,
            "expected Killed, got {:?}",
            task.status
        );
    }
}
