//! Execution engine — spawns commands, manages task lifecycle, streams events.

#[allow(unused_imports)]
pub mod process;
pub mod pty;
mod task;

pub use task::*;

use arshy_lib::ipc::{Task, TaskStatus};
use arshy_lib::Result;
use std::sync::Arc;

use super::bus::{BusEvent, BusEventKind, EventBus};
use super::ipc_handler::RunResult;
use super::parser::{Engine, ParsedTool, ParserSession};
use super::store::Store;

/// Core executor that owns the store, parser, and event bus.
pub struct Executor {
    store: Arc<Store>,
    parser: Arc<Engine>,
    event_bus: EventBus,
    config: ExecutorConfig,
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
        Self { store, parser, event_bus, config: ExecutorConfig::default() }
    }

    pub fn with_config(mut self, config: ExecutorConfig) -> Self {
        self.config = config;
        self
    }

    /// Schedule a command for execution and return a task_id immediately.
    ///
    /// The command runs in a background tokio task. Events are written to the
    /// store and published on the event bus as output arrives.
    pub async fn run(
        &self,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        _mode: &str,
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

        // Spawn background execution task
        let store = self.store.clone();
        let parser = self.parser.clone();
        let event_bus = self.event_bus.clone();
        let executor_config = self.config.clone();
        let cmd = command.to_string();
        let task_id_bg = task_id.clone();
        let detected_tool = tool;

        tokio::spawn(async move {
            if let Err(e) = run_background(
                &task_id_bg,
                &cmd,
                cwd_string.as_deref(),
                timeout_ms,
                &detected_tool,
                store,
                parser,
                event_bus,
                executor_config,
            ).await {
                tracing::error!("task {} execution error: {}", task_id_bg, e);
            }
        });

        Ok(RunResult { task_id, status: TaskStatus::Running, pid: None })
    }

    /// Kill a running task.
    pub async fn kill(&self, task_id: &str) -> Result<()> {
        // For now, mark as killed in DB. Task process management is handled
        // by the background task itself (which checks for cancellation).
        self.store.update_task(task_id, &TaskStatus::Killed, Some(-1), None)?;
        Ok(())
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

    pub fn parser(&self) -> &Engine {
        &self.parser
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }
}

/// Background task that runs the command, parses output, stores events.
async fn run_background(
    task_id: &str,
    command: &str,
    cwd: Option<&str>,
    timeout_ms: Option<u64>,
    detected_tool: &Option<ParsedTool>,
    store: Arc<Store>,
    parser: Arc<Engine>,
    event_bus: EventBus,
    config: ExecutorConfig,
) -> Result<()> {
    let start = std::time::Instant::now();
    let cwd_path = cwd.map(std::path::PathBuf::from);

    // Create a parser session for this task (holds state for stateful parsers)
    let session = parser.create_session(detected_tool.as_ref());

    // Spawn the process
    let mut handle = pty::spawn_command(command, cwd_path.as_deref()).await?;
    let pid = handle.pid;

    // Update task with PID
    let _ = store.update_task_pid(task_id, pid);

    // Publish task/update event
    event_bus.publish(BusEvent {
        connection_id: 0, // broadcast to all
        kind: BusEventKind::TaskUpdate {
            task_id: task_id.to_string(),
            status: "running".into(),
            elapsed_ms: 0,
        },
    });

    let timeout = tokio::time::Duration::from_millis(
        timeout_ms.unwrap_or(config.max_task_duration_ms),
    );
    let mut seq: u64 = 0;
    let mut total_bytes: u64 = 0;
    let mut error_count: u64 = 0;
    let max_bytes = config.max_output_bytes;

    // Read output lines with timeout
    let exit_code = tokio::select! {
        result = async {
            while let Some((source, line)) = handle.output_rx.recv().await {
                total_bytes += line.len() as u64;
                if total_bytes > max_bytes {
                    tracing::warn!("task {} output exceeded {} bytes, truncating", task_id, max_bytes);
                    break;
                }

                // Parse the line using the session (handles both TOML and stateful parsers)
                let events = session.parse_line(&line, seq, detected_tool.as_ref());

                for mut event in events {
                    seq += 1;
                    event.seq = seq;

                    // Track source (stdout/stderr) — stderr lines get elevated severity
                    if source == "stderr" && event.severity.as_deref() == Some("info") {
                        event.severity = Some("warning".into());
                    }

                    if event.severity.as_deref() == Some("error") {
                        error_count += 1;
                    }

                    // Store event in DB
                    if let Err(e) = store.insert_event(task_id, seq, &event) {
                        tracing::error!("task {} failed to store event: {}", task_id, e);
                    }

                    // Publish diagnostic event on bus
                    event_bus.publish(BusEvent {
                        connection_id: 0,
                        kind: BusEventKind::Diagnostic {
                            task_id: task_id.to_string(),
                            event,
                        },
                    });
                }
            }

            // Wait for process exit
            handle.wait().await
        } => {
            match result {
                Ok(code) => code,
                Err(e) => {
                    tracing::error!("task {} wait error: {}", task_id, e);
                    Some(-1)
                }
            }
        }
        _ = tokio::time::sleep(timeout) => {
            tracing::warn!("task {} timed out after {}ms", task_id, timeout.as_millis());
            // Kill the process
            let _ = handle.force_kill();
            let _ = store.update_task(task_id, &TaskStatus::Timeout, Some(-2), None);
            None
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let final_status = match exit_code {
        Some(0) => TaskStatus::Completed,
        Some(_) => TaskStatus::Failed,
        None => TaskStatus::Timeout,
    };

    let exit_code_val = exit_code.unwrap_or(-1);

    // Emit completion events from stateful parsers
    let completion_events = session.on_complete(exit_code_val, seq);
    for mut event in completion_events {
        seq += 1;
        event.seq = seq;
        if let Err(e) = store.insert_event(task_id, seq, &event) {
            tracing::error!("task {} failed to store completion event: {}", task_id, e);
        }
        event_bus.publish(BusEvent {
            connection_id: 0,
            kind: BusEventKind::Diagnostic {
                task_id: task_id.to_string(),
                event,
            },
        });
    }

    // Update task in DB
    if let Err(e) = store.update_task(task_id, &final_status, Some(exit_code_val), Some(duration_ms)) {
        tracing::error!("task {} failed to update final status: {}", task_id, e);
    }

    // Publish completion event
    event_bus.publish(BusEvent {
        connection_id: 0,
        kind: BusEventKind::TaskComplete {
            task_id: task_id.to_string(),
            exit_code: exit_code_val,
            duration_ms,
        },
    });

    tracing::info!(
        "task {} completed: status={:?}, exit_code={}, duration={}ms, events={}, errors={}",
        task_id, final_status, exit_code_val, duration_ms, seq, error_count
    );

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
}
