//! Execution engine — spawns commands, manages task lifecycle, streams events.
//!
//! Execution modes:
//! - **sync**: Wait for completion, return full result (blocking).
//! - **async**: Return immediately with task_id, stream events via notifications.
//! - **auto**: Smart — short commands get zero-overhead sync path,
//!   long commands get async + structured output.

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
use super::security::{AuditEntry, AuditLog, CommandFilter};
use super::store::Store;

/// Determine whether a command is "short" — eligible for zero-overhead sync path.
///
/// Short commands skip store insertion, parser session, and event streaming.
/// They return raw stdout directly, matching the experience of a native shell tool.
pub fn is_short_command(command: &str) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() {
        return true;
    }
    if cmd.len() > 80 {
        return false;
    }
    // Pipes, redirects, chaining, backgrounding → non-short
    if cmd.contains('|') || cmd.contains(">>") || cmd.contains("&&")
        || cmd.contains("||") || cmd.contains('&')
    {
        return false;
    }
    // Long-running flags → non-short
    let long_flags = ["--watch", "-f", "serve", "daemon", "start", "dev", "preview"];
    if long_flags.iter().any(|f| cmd.contains(f)) {
        return false;
    }
    cmd.split_whitespace().count() <= 5
}

/// Core executor that owns the store, parser, and event bus.
pub struct Executor {
    store: Arc<Store>,
    parser: Arc<Engine>,
    event_bus: EventBus,
    config: ExecutorConfig,
    filter: CommandFilter,
    sandbox_paths: Vec<String>,
    access_level: String,
    audit_log: Option<Arc<AuditLog>>,
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
            filter: CommandFilter::permissive(),
            sandbox_paths: Vec::new(),
            access_level: "full".into(),
            audit_log: None,
            kill_registry: Arc::new(TokioMutex::new(HashMap::new())),
        }
    }

    pub fn with_config(mut self, config: ExecutorConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_security(mut self, config: &arshy_lib::config::SecurityConfig) -> Self {
        self.filter = CommandFilter::from_config(config);
        self.sandbox_paths = config.sandbox_paths.clone();
        self.access_level = config.access_level.clone();
        self
    }

    pub fn with_audit_log(mut self, audit_log: Arc<AuditLog>) -> Self {
        self.audit_log = Some(audit_log);
        self
    }

    /// Current access level ("full" or "read-only").
    pub fn access_level(&self) -> &str {
        &self.access_level
    }

    /// Schedule a command for execution.
    ///
    /// Mode behavior:
    /// - **sync**: Wait for completion, full structured path.
    /// - **async**: Return immediately with task_id, events stream via notifications.
    /// - **auto**: Smart — short commands get zero-overhead sync path (raw stdout),
    ///   long commands get async + structured path.
    pub async fn run(
        &self,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        mode: &str,
        parse_hint: Option<&str>,
    ) -> Result<RunResult> {
        // ── Security checks (always run) ──────────────────────────────────
        if let Err(e) = self.filter.check(command) {
            if let Some(ref audit) = self.audit_log {
                let _ = audit.log(&AuditEntry {
                    timestamp: chrono::Utc::now(),
                    task_id: String::new(),
                    command: command.to_string(),
                    cwd: cwd.map(String::from),
                    exit_code: None,
                    blocked: true,
                    reason: Some(e.to_string()),
                });
            }
            return Err(e);
        }

        super::security::check_path(
            cwd.unwrap_or("."),
            &self.sandbox_paths,
        )?;

        let is_auto = mode == "auto";
        let is_explicit_sync = mode == "sync";
        let is_short = is_short_command(command);
        // When the agent provides a parse_hint, it expects structured output —
        // bypass the zero-overhead short path to ensure parser processing.
        let has_hint = parse_hint.is_some();

        // ── Auto + short (no hint) → zero-overhead fast path ──────────────
        if is_auto && is_short && !has_hint {
            return self.run_short(command, cwd, timeout_ms).await;
        }

        // ── Full structured path ──────────────────────────────────────────
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

        // Auto → async for long commands; explicit sync/async → as-is
        let is_sync = is_explicit_sync;

        // Spawn background execution task
        let store = self.store.clone();
        let parser = self.parser.clone();
        let event_bus = self.event_bus.clone();
        let executor_config = self.config.clone();
        let cmd = command.to_string();
        let task_id_bg = task_id.clone();
        let detected_tool = tool;

        let (done_tx, done_rx) = if is_sync {
            let (tx, rx) = oneshot::channel::<CompletionInfo>();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

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
            audit_log: self.audit_log.clone(),
            done_tx,
            kill_rx,
        };
        let task_id_cleanup = task_id.clone();
        tokio::spawn(async move {
            if let Err(e) = run_background(task).await {
                tracing::error!("background task failed: {}", e);
            }
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
                raw_output: None,
                short_command: false,
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
                    raw_output: None,
                    short_command: false,
                }),
                Err(_) => Ok(RunResult {
                    task_id,
                    status: TaskStatus::Failed,
                    pid: None,
                    exit_code: Some(-1),
                    duration_ms: None,
                    event_count: None,
                    error_count: None,
                    raw_output: None,
                    short_command: false,
                }),
            },
            None => unreachable!(),
        }
    }

    /// Zero-overhead fast path for short commands.
    ///
    /// Skips Store insert, parser session, EventBus — directly spawns, waits,
    /// and returns raw stdout. Security checks and audit logging still apply.
    async fn run_short(
        &self,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> Result<RunResult> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let start = std::time::Instant::now();

        let cwd_path = cwd.map(std::path::PathBuf::from);
        let mut handle = pty::spawn_command(command, cwd_path.as_deref()).await?;
        let pid = handle.pid;

        let timeout_dur = tokio::time::Duration::from_millis(
            timeout_ms.unwrap_or(self.config.max_task_duration_ms),
        );

        // Collect stdout (discard stderr for short commands)
        let mut stdout_lines: Vec<String> = Vec::new();
        let timed_out = tokio::select! {
            _result = async {
                while let Some((_source, line)) = handle.output_rx.recv().await {
                    stdout_lines.push(line);
                }
            } => Ok(()),
            _ = tokio::time::sleep(timeout_dur) => {
                let _ = handle.force_kill();
                Err(())
            }
        };

        let exit_code = match handle.wait().await {
            Ok(code) => code.unwrap_or(-1),
            Err(_) => -1,
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let status = if timed_out.is_err() {
            TaskStatus::Timeout
        } else {
            match exit_code {
                0 => TaskStatus::Completed,
                _ => TaskStatus::Failed,
            }
        };

        let raw_output = stdout_lines.join("\n");

        // Audit log
        if let Some(ref audit) = self.audit_log {
            let _ = audit.log(&AuditEntry {
                timestamp: chrono::Utc::now(),
                task_id: task_id.clone(),
                command: command.to_string(),
                cwd: cwd.map(String::from),
                exit_code: Some(exit_code),
                blocked: false,
                reason: None,
            });
        }

        Ok(RunResult {
            task_id,
            status,
            pid: Some(pid),
            exit_code: Some(exit_code),
            duration_ms: Some(duration_ms),
            event_count: None,
            error_count: None,
            raw_output: Some(raw_output),
            short_command: true,
        })
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
    audit_log: Option<Arc<AuditLog>>,
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
            let mut full_output = String::new();
            while let Some((source, line)) = handle.output_rx.recv().await {
                full_output.push_str(&line);
                full_output.push('\n');
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

                    if source == "stderr" {
                        match event.severity.as_deref() {
                            Some("info") => {
                                if super::parser::stderr_looks_like_error(&line) {
                                    event.severity = Some("error".into());
                                } else {
                                    event.severity = Some("warning".into());
                                }
                            }
                            Some("warning") if super::parser::stderr_looks_like_error(&line) => {
                                event.severity = Some("error".into());
                            }
                            _ => {}
                        }
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
            // Try JSON parsing on the full accumulated output
            if let Some(json_events) = super::parser::try_parse_json(&full_output) {
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

        let result = executor.run("echo hello", None, None, "async", None).await.unwrap();
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

        let result = executor.run("exit 1", None, None, "async", None).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.exit_code, Some(1));
    }

    #[tokio::test]
    async fn test_executor_tail() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("printf 'a\nb\nc\n'", None, None, "async", None).await.unwrap();

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

        let result = executor.run("sleep 60", None, Some(500), "async", None).await.unwrap();

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

        let result = executor.run("echo sync_test", None, None, "sync", None).await.unwrap();
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

        let result = executor.run("exit 42", None, None, "sync", None).await.unwrap();
        assert_eq!(result.status, TaskStatus::Failed);
        assert_eq!(result.exit_code, Some(42));
    }

    #[tokio::test]
    async fn test_executor_kill_graceful() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("sleep 60", None, None, "async", None).await.unwrap();
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

    // ── P11: Auto mode tests ──────────────────────────────────────────────────

    /// is_short_command() edge cases
    #[test]
    fn short_command_empty() {
        assert!(is_short_command(""));
        assert!(is_short_command("   "));
    }

    #[test]
    fn short_command_under_80_chars() {
        assert!(is_short_command("ls -la"));
        assert!(is_short_command("echo hello world"));
        assert!(is_short_command("git status"));
    }

    #[test]
    fn short_command_over_80_chars() {
        let long = "echo this is a really really really really really really really long command that exceeds eighty characters easily";
        assert!(!is_short_command(long));
    }

    #[test]
    fn short_command_has_pipe() {
        assert!(!is_short_command("ls -la | grep foo"));
        assert!(!is_short_command("cat file.txt | head -5"));
    }

    #[test]
    fn short_command_has_redirect() {
        assert!(!is_short_command("echo hello >> out.txt"));
    }

    #[test]
    fn short_command_has_chaining() {
        assert!(!is_short_command("make build && make test"));
        assert!(!is_short_command("cd dir || exit 1"));
    }

    #[test]
    fn short_command_has_background() {
        assert!(!is_short_command("npm run dev &"));
    }

    #[test]
    fn short_command_too_many_words() {
        assert!(!is_short_command("one two three four five six"));
    }

    #[test]
    fn short_command_long_flag_detected() {
        assert!(!is_short_command("cargo watch --watch src/"));
        assert!(!is_short_command("tail -f /var/log/system.log"));
        assert!(!is_short_command("python -m http.server 8080"));
    }

    #[test]
    fn short_command_daemon_flag() {
        assert!(!is_short_command("nginx daemon off"));
    }

    /// Auto mode with a short command returns raw_output + short_command flag.
    #[tokio::test]
    async fn auto_short_returns_raw_output() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("echo fast-path", None, None, "auto", None).await.unwrap();
        assert!(result.short_command, "short_command should be true");
        assert_eq!(result.status, TaskStatus::Completed);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.raw_output.is_some(), "raw_output should be populated");
        assert!(
            result.raw_output.as_ref().unwrap().contains("fast-path"),
            "raw_output should contain the command's stdout"
        );
        assert!(result.duration_ms.is_some());
    }

    /// Auto mode with a long command (over 80 chars) takes the async path.
    #[tokio::test]
    async fn auto_long_takes_async_path() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run(
            "echo this-command-is-definitely-longer-than-eighty-characters-so-it-should-trigger-async-path-xxxxxxxxx",
            None,
            None,
            "auto",
            None,
        ).await.unwrap();
        assert!(!result.short_command, "long command should not be short_command");
        assert_eq!(result.status, TaskStatus::Running);
        assert!(result.raw_output.is_none(), "async path should not populate raw_output");

        // Wait for background task completion
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
    }

    /// Auto mode with a piped command takes the async path.
    #[tokio::test]
    async fn auto_piped_takes_async_path() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("echo hello | cat", None, None, "auto", None).await.unwrap();
        assert!(!result.short_command, "piped command should not be short");
        assert_eq!(result.status, TaskStatus::Running);

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
    }

    /// Explicit sync mode with a short command still uses the full structured path.
    #[tokio::test]
    async fn sync_with_short_uses_full_path() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("echo sync-short", None, None, "sync", None).await.unwrap();
        // Sync mode: should complete and return structure, not short path
        assert!(!result.short_command, "explicit sync should use full structured path");
        assert_eq!(result.status, TaskStatus::Completed);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.event_count.unwrap() >= 1, "should have stored events");
        assert!(result.raw_output.is_none(), "full path should not set raw_output");
    }

    /// Auto + short + failure: raw_output still populated, status is Failed.
    #[tokio::test]
    async fn auto_short_failure_has_raw_output() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("nonexistent_xyz", None, None, "auto", None).await.unwrap();
        assert!(result.short_command);
        assert_eq!(result.status, TaskStatus::Failed);
        assert!(result.exit_code.unwrap() != 0);
        assert!(result.raw_output.is_some());
    }

    // ── S2+S4: parse_hint + mode:auto linkage ────────────────────────────────

    /// parse_hint="json" bypasses the short path to ensure structured processing.
    #[tokio::test]
    async fn parse_hint_json_forces_structured_path() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        // Short command with parse_hint="json" → should NOT take short path
        let result = executor.run("echo hello", None, None, "auto", Some("json")).await.unwrap();
        assert!(!result.short_command, "parse_hint should force structured path");
        assert_eq!(result.status, TaskStatus::Running);

        // Wait for background completion
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
    }

    /// parse_hint="json" with JSON output command produces structured events.
    #[tokio::test]
    async fn parse_hint_json_with_json_output() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run(
            r#"echo '{"status":"ok","count":1}'"#,
            None,
            None,
            "auto",
            Some("json"),
        ).await.unwrap();
        assert!(!result.short_command);

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Verify JSON events were stored
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
        let (events, _total) = store.query_events(&params).unwrap();
        // Should have at least one JSON data event
        let has_json_event = events.iter().any(|e| e.event_type == "data");
        assert!(has_json_event, "expected at least one JSON data event");
    }

    /// parse_hint="raw" also forces structured path (any hint forces it).
    #[tokio::test]
    async fn parse_hint_raw_forces_structured_path() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        // Short command with parse_hint="raw" → structured path
        let result = executor.run("echo hello", None, None, "auto", Some("raw")).await.unwrap();
        assert!(!result.short_command, "any parse_hint should force structured path");
        assert_eq!(result.status, TaskStatus::Running);

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
    }

    /// No parse_hint → short commands take the fast path (existing behavior).
    #[tokio::test]
    async fn no_parse_hint_keeps_short_path() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run("echo fast", None, None, "auto", None).await.unwrap();
        assert!(result.short_command, "no hint should keep short path for short commands");
        assert!(result.raw_output.is_some());
    }

    /// RunTaskParams deserializes parse_hint correctly.
    #[test]
    fn runtaskparams_default_parse_hint() {
        let json = r#"{"command":"ls"}"#;
        let params: arshy_lib::ipc::RunTaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.command, "ls");
        assert_eq!(params.mode, "auto");
        assert!(params.parse_hint.is_none());
    }

    #[test]
    fn runtaskparams_with_parse_hint() {
        let json = r#"{"command":"gh pr list --json","parse_hint":"json"}"#;
        let params: arshy_lib::ipc::RunTaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.parse_hint.as_deref(), Some("json"));
    }

    // ── S6: CLI+Skill adaptation tests ───────────────────────────────────────

    /// JSON output via echo → JSON parser auto-detects at completion.
    #[tokio::test]
    async fn cli_json_output_auto_detected() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run(
            r#"printf '{"name":"test","count":42}\n'"#,
            None,
            None,
            "async",
            None,
        ).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

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
        let (events, _total) = store.query_events(&params).unwrap();
        let json_events: Vec<_> = events.iter().filter(|e| e.event_type == "data").collect();
        assert!(!json_events.is_empty(), "JSON output should produce data events");
        assert!(
            json_events.iter().any(|e| e.message.contains("name") && e.message.contains("test")),
            "JSON data event should contain the parsed content"
        );
    }

    /// parse_hint="json" with a JSON array → produces one event per array element.
    #[tokio::test]
    async fn parse_hint_json_array_produces_structured_events() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run(
            r#"echo '["item-a","item-b","item-c"]'"#,
            None,
            None,
            "auto",
            Some("json"),
        ).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

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
        let (events, _total) = store.query_events(&params).unwrap();
        let json_events: Vec<_> = events.iter().filter(|e| e.event_type == "data").collect();
        assert_eq!(json_events.len(), 3, "JSON array of 3 items should produce 3 data events");
    }

    /// CLI without dedicated parser → stderr error recognition catches common error patterns.
    #[tokio::test]
    async fn stderr_recognizes_generic_errors() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        // Write to stderr with common error patterns
        let result = executor.run(
            r#"sh -c 'echo "error: cannot find module" >&2; echo "warning: using fallback" >&2; exit 0'"#,
            None,
            None,
            "async",
            None,
        ).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

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
        let (events, _total) = store.query_events(&params).unwrap();

        let has_error = events.iter().any(|e| e.severity.as_deref() == Some("error"));
        let has_warning = events.iter().any(|e| e.severity.as_deref() == Some("warning"));
        assert!(has_error, "stderr with 'error:' should produce error severity");
        assert!(has_warning, "stderr with 'warning:' should produce warning severity");
    }

    /// stderr with "Permission denied" is detected as error.
    #[tokio::test]
    async fn stderr_permission_denied_is_error() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        let result = executor.run(
            r#"sh -c 'echo "Permission denied (os error 13)" >&2; exit 1'"#,
            None,
            None,
            "async",
            None,
        ).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

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
        let (events, _total) = store.query_events(&params).unwrap();
        assert!(
            events.iter().any(|e| e.severity.as_deref() == Some("error")),
            "Permission denied on stderr should be classified as error"
        );
    }

    /// Short command + parse_hint → structured path with events in store.
    #[tokio::test]
    async fn short_command_with_parse_hint_stores_events() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        // Short command with parse_hint → forced structured path
        let result = executor.run(
            "echo structured",
            None,
            None,
            "auto",
            Some("raw"),
        ).await.unwrap();

        assert!(!result.short_command);

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let task = store.get_task(&result.task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.events_count > 0, "structured path should store events");
    }

    /// Non-JSON output with no parse_hint → no JSON parsing attempted (graceful fallthrough).
    #[tokio::test]
    async fn non_json_output_no_false_positive() {
        let (store, parser, bus, _tmp) = setup();
        let executor = Executor::new(store.clone(), parser, bus);

        // Plain text output
        let result = executor.run(
            "printf 'regular output\nmore output\n'",
            None,
            None,
            "async",
            None,
        ).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

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
        let (events, _total) = store.query_events(&params).unwrap();
        // All should be "log" type, no "data" (JSON) events
        assert!(
            events.iter().all(|e| e.event_type != "data"),
            "plain text output should not produce JSON data events"
        );
        assert!(
            events.iter().all(|e| e.event_type == "log"),
            "plain text output should produce log events"
        );
    }
}
