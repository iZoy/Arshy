//! Execution engine — spawns commands in PTY, manages task lifecycle.

#[allow(unused_imports)]
mod process;
mod pty;
mod task;

#[allow(unused_imports)]
pub use process::*;
pub use pty::*;
pub use task::*;

use arshy_lib::Result;
use arshy_lib::ipc::{Task, TaskStatus};
use std::sync::Arc;

use super::bus::EventBus;
use super::ipc_handler::RunResult;
use super::parser::Engine;
use super::store::Store;

/// Core executor that owns the store, parser, and event bus.
pub struct Executor {
    store: Arc<Store>,
    parser: Arc<Engine>,
    event_bus: EventBus,
}

impl Executor {
    pub fn new(store: Arc<Store>, parser: Arc<Engine>, event_bus: EventBus) -> Self {
        Self { store, parser, event_bus }
    }

    /// Schedule a command for execution and return a task_id immediately.
    pub async fn run(
        &self,
        command: &str,
        cwd: Option<&str>,
        _timeout_ms: Option<u64>,
        _mode: &str,
    ) -> Result<RunResult> {
        let tool = self.parser.detect(command);

        let task_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let task = Task {
            task_id: task_id.clone(),
            command: command.to_string(),
            cwd: cwd.map(String::from),
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

        Ok(RunResult { task_id, status: TaskStatus::Running, pid: None })
    }

    /// Kill a running task (stub — wired when PTY is implemented).
    pub async fn kill(&self, task_id: &str) -> Result<()> {
        self.store.update_task(task_id, &TaskStatus::Killed, Some(-1), None)?;
        Ok(())
    }

    /// Tail the most recent output of a task (stub — wired when exec loop runs).
    pub async fn tail(&self, _task_id: &str, _lines: usize, _format: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }

    pub fn parser(&self) -> &Engine {
        &self.parser
    }

    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }
}
