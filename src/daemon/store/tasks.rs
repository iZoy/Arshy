use arshy_lib::ipc::{Task, TaskStatus};
use arshy_lib::Result;

impl super::Store {
    /// Insert a new task record.
    pub fn insert_task(&self, task: &Task) -> Result<()> {
        let mut tasks = self.lock();
        if tasks.contains_key(&task.task_id) {
            return Err(arshy_lib::ArshyError::Other(format!(
                "duplicate task_id: {}",
                task.task_id
            )));
        }
        let record = super::TaskRecord {
            task: task.clone(),
            raw_output: None,
            dedup_collapsed: 0,
            correlated_errors: 0,
        };
        tasks.insert(task.task_id.clone(), record);
        drop(tasks);
        self.persist_tasks()
    }

    /// List tasks, optionally filtered by status.
    pub fn list_tasks(&self, status: Option<&str>, limit: usize) -> Result<Vec<Task>> {
        let tasks = self.lock();
        let mut filtered: Vec<&super::TaskRecord> = if let Some(s) = status {
            // Strip surrounding quotes from the status filter
            let trimmed = s.trim_matches('"');
            tasks.values().filter(|r| status_matches(&r.task.status, trimmed)).collect()
        } else {
            tasks.values().collect()
        };

        // Sort by started_at DESC
        filtered.sort_by(|a, b| b.task.started_at.cmp(&a.task.started_at));

        Ok(filtered.into_iter().take(limit).map(|r| r.task.clone()).collect())
    }

    /// Get a single task by ID.
    #[allow(dead_code)] // used by tests and exec layer
    pub fn get_task(&self, task_id: &str) -> Result<Option<Task>> {
        let tasks = self.lock();
        Ok(tasks.get(task_id).map(|r| r.task.clone()))
    }

    /// Update task status.
    pub fn update_task(
        &self,
        task_id: &str,
        status: &TaskStatus,
        exit_code: Option<i32>,
        duration_ms: Option<u64>,
    ) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.task.status = status.clone();
            record.task.exit_code = exit_code;
            record.task.duration_ms = duration_ms;
            record.task.finished_at = Some(chrono::Utc::now().to_rfc3339());
        }
        drop(tasks);
        self.persist_tasks()
    }

    /// Update task PID (called after process spawn).
    pub fn update_task_pid(&self, task_id: &str, pid: u32) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.task.pid = Some(pid);
        }
        drop(tasks);
        self.persist_tasks()
    }

    /// Store full raw output for a task.
    pub fn update_task_raw_output(&self, task_id: &str, raw_output: &str) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.raw_output = Some(raw_output.to_string());
        }
        drop(tasks);
        self.persist_tasks()
    }

    /// Retrieve full raw output for a task.
    #[allow(dead_code)] // used by future tail --format raw command
    pub fn get_task_raw_output(&self, task_id: &str) -> Result<Option<String>> {
        let tasks = self.lock();
        Ok(tasks.get(task_id).and_then(|r| r.raw_output.clone()))
    }

    /// Increment feature usage counters for a completed task.
    /// Uses additive updates so multiple calls accumulate correctly.
    pub fn update_task_counters(
        &self,
        task_id: &str,
        dedup_collapsed: u64,
        correlated_errors: u64,
    ) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.dedup_collapsed += dedup_collapsed;
            record.correlated_errors += correlated_errors;
        }
        drop(tasks);
        self.persist_tasks()
    }
}

/// Check if a task status matches a filter string.
fn status_matches(status: &TaskStatus, filter: &str) -> bool {
    match status {
        TaskStatus::Running => filter == "running",
        TaskStatus::Completed => filter == "completed",
        TaskStatus::Failed => filter == "failed",
        TaskStatus::Killed => filter == "killed",
        TaskStatus::Timeout => filter == "timeout",
    }
}
