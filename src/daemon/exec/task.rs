use arshy_lib::ipc::TaskStatus;

/// Internal mutable state for a running task.
#[allow(dead_code)]
pub struct TaskState {
    pub task_id: String,
    pub status: TaskStatus,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

impl TaskState {
    pub fn new(task_id: String) -> Self {
        Self {
            task_id,
            status: TaskStatus::Running,
            pid: None,
            exit_code: None,
            started_at: chrono::Utc::now(),
        }
    }

    pub fn elapsed_ms(&self) -> i64 {
        (chrono::Utc::now() - self.started_at).num_milliseconds()
    }
}
