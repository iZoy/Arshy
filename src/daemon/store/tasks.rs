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

    /// Get a task's raw output byte count without cloning the whole task.
    /// Used by the executor to compute honest agent_delivered_bytes against
    /// the actual raw size.
    #[allow(dead_code)]
    pub fn get_task_raw_output_bytes(&self, task_id: &str) -> u64 {
        self.lock().get(task_id).map(|r| r.metrics.raw_output_bytes).unwrap_or(0)
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

    /// Set agent delivered bytes metric on a task record.
    pub fn update_task_agent_delivered_bytes(&self, task_id: &str, bytes: u64) -> Result<()> {
        let mut tasks = self.lock();
        if let Some(record) = tasks.get_mut(task_id) {
            record.metrics.agent_delivered_bytes = bytes;
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

#[cfg(test)]
mod tests {
    use crate::daemon::store::Store;
    use crate::ipc::{Task, TaskStatus};
    use tempfile::TempDir;

    fn test_store() -> (Store, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        store.initialize_schema().unwrap();
        (store, tmp)
    }

    fn make_task(id: &str, status: TaskStatus) -> Task {
        Task {
            task_id: id.to_string(),
            command: format!("echo {id}"),
            cwd: Some("/tmp".to_string()),
            status,
            exit_code: None,
            pid: None,
            parser_name: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            duration_ms: None,
            events_count: 0,
            error_count: 0,
            purpose: None,
            carrier: None,
        }
    }

    #[test]
    fn insert_and_get_task_round_trip() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("t1", TaskStatus::Running)).unwrap();
        let got = store.get_task("t1").unwrap().expect("task present");
        assert_eq!(got.task_id, "t1");
        assert_eq!(got.status, TaskStatus::Running);
    }

    #[test]
    fn insert_duplicate_task_errors() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("dup", TaskStatus::Running)).unwrap();
        let err = store.insert_task(&make_task("dup", TaskStatus::Running));
        assert!(err.is_err());
    }

    #[test]
    fn get_missing_task_returns_none() {
        let (store, _t) = test_store();
        assert!(store.get_task("nope").unwrap().is_none());
    }

    #[test]
    fn update_task_status_and_exit_code() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("t1", TaskStatus::Running)).unwrap();
        store.update_task("t1", &TaskStatus::Completed, Some(0), Some(123)).unwrap();
        let got = store.get_task("t1").unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Completed);
        assert_eq!(got.exit_code, Some(0));
        assert_eq!(got.duration_ms, Some(123));
        assert!(got.finished_at.is_some());
    }

    #[test]
    fn update_task_pid() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("t1", TaskStatus::Running)).unwrap();
        store.update_task_pid("t1", 4242).unwrap();
        assert_eq!(store.get_task("t1").unwrap().unwrap().pid, Some(4242));
    }

    #[test]
    fn raw_output_round_trip() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("t1", TaskStatus::Completed)).unwrap();
        store.update_task_raw_output("t1", "hello raw output").unwrap();
        let out = store.get_task_raw_output("t1").unwrap();
        assert_eq!(out.as_deref(), Some("hello raw output"));
    }

    #[test]
    fn detected_tool_set_and_get() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("t1", TaskStatus::Completed)).unwrap();
        assert!(store.get_detected_tool("t1").is_none());
        store.set_detected_tool("t1", "cargo").unwrap();
        assert_eq!(store.get_detected_tool("t1").as_deref(), Some("cargo"));
    }

    #[test]
    fn enrichment_markers() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("t1", TaskStatus::Completed)).unwrap();
        assert!(!store.is_enriched("t1"));
        store.mark_enriched("t1").unwrap();
        assert!(store.is_enriched("t1"));
        // marking a non-existent task is a no-op but must not error
        assert!(store.mark_enriched("ghost").is_ok());
    }

    #[test]
    fn list_tasks_filter_by_status() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("a", TaskStatus::Completed)).unwrap();
        store.insert_task(&make_task("b", TaskStatus::Running)).unwrap();
        store.insert_task(&make_task("c", TaskStatus::Failed)).unwrap();

        let completed = store.list_tasks(Some("completed"), 100).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].task_id, "a");

        let running = store.list_tasks(Some("running"), 100).unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].task_id, "b");

        let all = store.list_tasks(None, 100).unwrap();
        assert_eq!(all.len(), 3);

        let limited = store.list_tasks(None, 2).unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn metric_counters_update_without_error() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("t1", TaskStatus::Completed)).unwrap();
        assert!(store.update_task_counters("t1", 2, 1).is_ok());
        assert!(store.update_task_raw_output_bytes("t1", 1024).is_ok());
        assert!(store.update_task_agent_delivered_bytes("t1", 512).is_ok());
        assert!(store.update_task_pairs_merged("t1", 3).is_ok());
    }
}
