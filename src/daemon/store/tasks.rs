use crate::ipc::{Task, TaskStatus};
use crate::Result;

impl super::Store {
    /// Insert a new task record.
    pub fn insert_task(&self, task: &Task) -> Result<()> {
        let mut tasks = self.lock();
        if tasks.contains_key(&task.task_id) {
            return Err(crate::ArshyError::Other(format!("duplicate task_id: {}", task.task_id)));
        }
        let record = super::TaskRecord {
            task: task.clone(),
            raw_output: None,
            dedup_collapsed: 0,
            correlated_errors: 0,
            metrics: super::TaskMetrics::default(),
            enriched: false,
            detected_tool: None,
        };
        tasks.insert(task.task_id.clone(), record);
        drop(tasks);
        self.mark_dirty();
        Ok(())
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
        self.mark_dirty();
        Ok(())
    }

    /// Store full raw output for a task.
    /// Writes to a separate file `<store_dir>/raw/<task_id>.txt` instead of
    /// keeping it in the in-memory HashMap.
    pub fn update_task_raw_output(&self, task_id: &str, raw_output: &str) -> Result<()> {
        self.write_raw_output(task_id, raw_output)?;
        // Mark dirty so the task record (which now knows raw exists) gets persisted
        self.mark_dirty();
        Ok(())
    }

    /// Retrieve full raw output for a task.
    /// Reads from the per-task file on disk.
    #[allow(dead_code)] // used by future tail --format raw command
    pub fn get_task_raw_output(&self, task_id: &str) -> Result<Option<String>> {
        self.read_raw_output(task_id)
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
        self.mark_dirty();
        Ok(())
    }

    /// Set raw output bytes metric on a task record.
    pub fn update_task_raw_output_bytes(&self, task_id: &str, bytes: u64) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.metrics.raw_output_bytes = bytes;
        }
        drop(tasks);
        self.mark_dirty();
        Ok(())
    }

    /// Increment the pairs_merged metric on a task record.
    pub fn update_task_pairs_merged(&self, task_id: &str, count: u64) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.metrics.pairs_merged += count;
        }
        drop(tasks);
        self.mark_dirty();
        Ok(())
    }

    /// Set the detected tool name for a task (used by async enrichment).
    pub fn set_detected_tool(&self, task_id: &str, tool: &str) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.detected_tool = Some(tool.to_string());
        }
        drop(tasks);
        self.mark_dirty();
        Ok(())
    }

    /// Get the detected tool name for a task.
    pub fn get_detected_tool(&self, task_id: &str) -> Option<String> {
        let tasks = self.lock();
        tasks.get(task_id).and_then(|r| r.detected_tool.clone())
    }

    /// Check whether a task's events have already been enriched.
    pub fn is_enriched(&self, task_id: &str) -> bool {
        let tasks = self.lock();
        tasks.get(task_id).is_some_and(|r| r.enriched)
    }

    /// Mark a task as enriched (context + hints have been applied).
    /// Returns Ok(()) even if the task_id doesn't exist (no-op).
    pub fn mark_enriched(&self, task_id: &str) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.enriched = true;
        }
        drop(tasks);
        self.mark_dirty();
        Ok(())
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
