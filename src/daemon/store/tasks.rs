use arshy_lib::ipc::{Task, TaskStatus};
use arshy_lib::Result;

impl super::Store {
    /// Insert a new task record.
    pub fn insert_task(&self, task: &Task) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO tasks (task_id, command, cwd, status, exit_code, pid, parser_name,
             started_at, events_count, error_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                task.task_id,
                task.command,
                task.cwd,
                serde_json::to_string(&task.status).unwrap_or_default(),
                task.exit_code,
                task.pid,
                task.parser_name,
                task.started_at,
                task.events_count,
                task.error_count,
            ],
        )?;
        Ok(())
    }

    /// List tasks, optionally filtered by status.
    pub fn list_tasks(&self, status: Option<&str>, limit: usize) -> Result<Vec<Task>> {
        let conn = self.lock();
        let tasks = if let Some(s) = status {
            let mut stmt = conn.prepare(
                "SELECT task_id, command, cwd, status, exit_code, pid, parser_name,
                 started_at, finished_at, duration_ms, events_count, error_count
                 FROM tasks WHERE status=?1 ORDER BY started_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![s, limit as i64], map_task)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT task_id, command, cwd, status, exit_code, pid, parser_name,
                 started_at, finished_at, duration_ms, events_count, error_count
                 FROM tasks ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(rusqlite::params![limit as i64], map_task)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(tasks)
    }

    /// Get a single task by ID.
    #[allow(dead_code)] // used by tests and exec layer
    pub fn get_task(&self, task_id: &str) -> Result<Option<Task>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT task_id, command, cwd, status, exit_code, pid, parser_name,
             started_at, finished_at, duration_ms, events_count, error_count
             FROM tasks WHERE task_id=?1",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![task_id], map_task)?;
        match rows.next() {
            Some(Ok(t)) => Ok(Some(t)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Update task status.
    pub fn update_task(
        &self,
        task_id: &str,
        status: &TaskStatus,
        exit_code: Option<i32>,
        duration_ms: Option<u64>,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE tasks SET status=?1, exit_code=?2, duration_ms=?3, finished_at=?4
             WHERE task_id=?5",
            rusqlite::params![
                serde_json::to_string(status).unwrap_or_default(),
                exit_code,
                duration_ms,
                chrono::Utc::now().to_rfc3339(),
                task_id,
            ],
        )?;
        Ok(())
    }

    /// Update task PID (called after process spawn).
    pub fn update_task_pid(&self, task_id: &str, pid: u32) -> Result<()> {
        let conn = self.lock();
        conn.execute("UPDATE tasks SET pid=?1 WHERE task_id=?2", rusqlite::params![pid, task_id])?;
        Ok(())
    }

    /// Store full raw output for a task.
    pub fn update_task_raw_output(&self, task_id: &str, raw_output: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE tasks SET raw_output = ?1 WHERE task_id = ?2",
            rusqlite::params![raw_output, task_id],
        )?;
        Ok(())
    }

    /// Retrieve full raw output for a task.
    #[allow(dead_code)] // used by future tail --format raw command
    pub fn get_task_raw_output(&self, task_id: &str) -> Result<Option<String>> {
        let conn = self.lock();
        let result = conn.query_row(
            "SELECT raw_output FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| row.get::<_, Option<String>>(0),
        );
        match result {
            Ok(raw) => Ok(raw),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

fn map_task(row: &rusqlite::Row<'_>) -> std::result::Result<Task, rusqlite::Error> {
    let status_str: String = row.get(3)?;
    Ok(Task {
        task_id: row.get(0)?,
        command: row.get(1)?,
        cwd: row.get(2)?,
        status: serde_json::from_str(&status_str).unwrap_or(TaskStatus::Completed),
        exit_code: row.get(4)?,
        pid: row.get(5)?,
        parser_name: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        duration_ms: row.get(9)?,
        events_count: row.get(10)?,
        error_count: row.get(11)?,
    })
}
