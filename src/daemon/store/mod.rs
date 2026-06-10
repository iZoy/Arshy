//! SQLite storage — tasks, events, tool versions, parser registry.

mod events;
mod prune;
mod schema;
mod tasks;
mod versions;

use arshy_lib::Result;
use std::path::Path;
use std::sync::Mutex;

/// Thread-safe SQLite store wrapper.
pub struct Store {
    conn: Mutex<rusqlite::Connection>,
}

impl Store {
    /// Open (or create) the SQLite database at `path`.
    pub fn open(path: &Path, wal_mode: bool) -> Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        if wal_mode {
            conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        }
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Run `PRAGMA integrity_check` and return the result.
    ///
    /// Returns `Ok(())` if the database is healthy, or an error with details.
    pub fn integrity_check(&self) -> Result<String> {
        let conn = self.lock();
        let result: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        Ok(result)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.conn.lock().expect("store mutex poisoned")
    }
}

// ── Store Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::ipc::{EventLocation, QueryParams, Task, TaskEvent, TaskStatus};
    use tempfile::TempDir;

    /// Create a temporary store with initialized schema.
    fn test_store() -> (Store, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Store::open(&db_path, false).unwrap();
        store.initialize_schema().unwrap();
        (store, tmp)
    }

    fn make_task(id: &str, command: &str, status: TaskStatus) -> Task {
        Task {
            task_id: id.to_string(),
            command: command.to_string(),
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
        }
    }

    fn make_event(event_type: &str, severity: &str, message: &str) -> TaskEvent {
        TaskEvent {
            seq: 0,
            event_type: event_type.to_string(),
            severity: Some(severity.to_string()),
            code: None,
            message: message.to_string(),
            location: None,
            context: None,
            hint: None,
        }
    }

    // ── Schema tests ────────────────────────────────────────────────────────

    #[test]
    fn schema_idempotent() {
        let (store, _tmp) = test_store();
        // Calling initialize_schema twice should not error
        store.initialize_schema().unwrap();
    }

    #[test]
    fn integrity_check_ok() {
        let (store, _tmp) = test_store();
        let result = store.integrity_check().unwrap();
        assert_eq!(result, "ok");
    }

    // ── Task CRUD tests ─────────────────────────────────────────────────────

    #[test]
    fn insert_and_get_task() {
        let (store, _tmp) = test_store();
        let task = make_task("task-001", "echo hello", TaskStatus::Running);
        store.insert_task(&task).unwrap();

        let fetched = store.get_task("task-001").unwrap().unwrap();
        assert_eq!(fetched.task_id, "task-001");
        assert_eq!(fetched.command, "echo hello");
        assert_eq!(fetched.status, TaskStatus::Running);
        assert_eq!(fetched.cwd, Some("/tmp".to_string()));
    }

    #[test]
    fn get_task_not_found() {
        let (store, _tmp) = test_store();
        let result = store.get_task("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn insert_task_duplicate_errors() {
        let (store, _tmp) = test_store();
        let task = make_task("dup-1", "echo a", TaskStatus::Running);
        store.insert_task(&task).unwrap();
        // Second insert with same ID should error (PRIMARY KEY)
        let result = store.insert_task(&task);
        assert!(result.is_err());
    }

    #[test]
    fn list_tasks_empty() {
        let (store, _tmp) = test_store();
        let tasks = store.list_tasks(None, 10).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn list_tasks_ordered_by_started_at() {
        let (store, _tmp) = test_store();
        let mut t1 = make_task("t1", "first", TaskStatus::Running);
        t1.started_at = "2024-01-01T00:00:00Z".to_string();
        let mut t2 = make_task("t2", "second", TaskStatus::Running);
        t2.started_at = "2024-01-02T00:00:00Z".to_string();
        let mut t3 = make_task("t3", "third", TaskStatus::Running);
        t3.started_at = "2024-01-03T00:00:00Z".to_string();

        store.insert_task(&t1).unwrap();
        store.insert_task(&t2).unwrap();
        store.insert_task(&t3).unwrap();

        let tasks = store.list_tasks(None, 10).unwrap();
        assert_eq!(tasks.len(), 3);
        // DESC order: t3, t2, t1
        assert_eq!(tasks[0].task_id, "t3");
        assert_eq!(tasks[1].task_id, "t2");
        assert_eq!(tasks[2].task_id, "t1");
    }

    #[test]
    fn list_tasks_filter_by_status() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("r1", "echo a", TaskStatus::Running)).unwrap();
        store.insert_task(&make_task("c1", "echo b", TaskStatus::Completed)).unwrap();
        store.insert_task(&make_task("r2", "echo c", TaskStatus::Running)).unwrap();

        let running = store.list_tasks(Some("\"running\""), 10).unwrap();
        assert_eq!(running.len(), 2);

        let completed = store.list_tasks(Some("\"completed\""), 10).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].task_id, "c1");
    }

    #[test]
    fn list_tasks_respects_limit() {
        let (store, _tmp) = test_store();
        for i in 0..5 {
            store.insert_task(&make_task(&format!("t{}", i), "cmd", TaskStatus::Running)).unwrap();
        }
        let tasks = store.list_tasks(None, 3).unwrap();
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn update_task_status() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("u1", "echo done", TaskStatus::Running)).unwrap();

        store.update_task("u1", &TaskStatus::Completed, Some(0), Some(150)).unwrap();

        let task = store.get_task("u1").unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.exit_code, Some(0));
        assert_eq!(task.duration_ms, Some(150));
        assert!(task.finished_at.is_some());
    }

    #[test]
    fn update_task_pid() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("p1", "echo pid", TaskStatus::Running)).unwrap();

        store.update_task_pid("p1", 12345).unwrap();

        let task = store.get_task("p1").unwrap().unwrap();
        assert_eq!(task.pid, Some(12345));
    }

    #[test]
    fn update_nonexistent_task_no_error() {
        let (store, _tmp) = test_store();
        // UPDATE on nonexistent row silently affects 0 rows
        store.update_task("ghost", &TaskStatus::Failed, Some(1), None).unwrap();
    }

    // ── Event tests ─────────────────────────────────────────────────────────

    #[test]
    fn insert_and_query_event() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("evt-1", "cmd", TaskStatus::Running)).unwrap();

        let event = make_event("diagnostic", "error", "type mismatch at line 10");
        store.insert_event("evt-1", 1, &event).unwrap();

        let params = QueryParams {
            task_id: "evt-1".to_string(),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].message, "type mismatch at line 10");
        assert_eq!(events[0].event_type, "diagnostic");
        assert_eq!(events[0].severity, Some("error".to_string()));
    }

    #[test]
    fn insert_event_increments_counts() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("cnt-1", "cmd", TaskStatus::Running)).unwrap();

        // Insert info event
        store.insert_event("cnt-1", 1, &make_event("log", "info", "starting")).unwrap();
        // Insert error event
        store.insert_event("cnt-1", 2, &make_event("diagnostic", "error", "fail")).unwrap();
        // Insert another info event
        store.insert_event("cnt-1", 3, &make_event("log", "info", "done")).unwrap();

        let task = store.get_task("cnt-1").unwrap().unwrap();
        assert_eq!(task.events_count, 3);
        assert_eq!(task.error_count, 1);
    }

    #[test]
    fn insert_event_with_location() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("loc-1", "cmd", TaskStatus::Running)).unwrap();

        let mut event = make_event("diagnostic", "error", "undefined var");
        event.location =
            Some(EventLocation { file: "src/main.rs".to_string(), line: 42, column: Some(10) });
        store.insert_event("loc-1", 1, &event).unwrap();

        let params = QueryParams {
            task_id: "loc-1".to_string(),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
        };
        let (events, _) = store.query_events(&params).unwrap();
        let loc = events[0].location.as_ref().unwrap();
        assert_eq!(loc.file, "src/main.rs");
        assert_eq!(loc.line, 42);
        assert_eq!(loc.column, Some(10));
    }

    #[test]
    fn query_events_filter_by_type() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("f1", "cmd", TaskStatus::Running)).unwrap();
        store.insert_event("f1", 1, &make_event("diagnostic", "error", "err")).unwrap();
        store.insert_event("f1", 2, &make_event("summary", "info", "done")).unwrap();
        store.insert_event("f1", 3, &make_event("diagnostic", "warning", "warn")).unwrap();

        let params = QueryParams {
            task_id: "f1".to_string(),
            event_type: Some("diagnostic".to_string()),
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 2);
        assert!(events.iter().all(|e| e.event_type == "diagnostic"));
    }

    #[test]
    fn query_events_filter_by_severity() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("f2", "cmd", TaskStatus::Running)).unwrap();
        store.insert_event("f2", 1, &make_event("diagnostic", "error", "err")).unwrap();
        store.insert_event("f2", 2, &make_event("diagnostic", "warning", "warn")).unwrap();
        store.insert_event("f2", 3, &make_event("log", "info", "info")).unwrap();

        let params = QueryParams {
            task_id: "f2".to_string(),
            event_type: None,
            severity: Some("error".to_string()),
            code: None,
            file: None,
            limit: 100,
            offset: 0,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].severity, Some("error".to_string()));
    }

    #[test]
    fn query_events_filter_by_type_and_severity() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("f3", "cmd", TaskStatus::Running)).unwrap();
        store.insert_event("f3", 1, &make_event("diagnostic", "error", "e1")).unwrap();
        store.insert_event("f3", 2, &make_event("diagnostic", "warning", "w1")).unwrap();
        store.insert_event("f3", 3, &make_event("summary", "error", "e2")).unwrap();

        let params = QueryParams {
            task_id: "f3".to_string(),
            event_type: Some("diagnostic".to_string()),
            severity: Some("error".to_string()),
            code: None,
            file: None,
            limit: 100,
            offset: 0,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].message, "e1");
    }

    #[test]
    fn query_events_pagination() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("pg1", "cmd", TaskStatus::Running)).unwrap();
        for i in 0..10 {
            store
                .insert_event("pg1", i, &make_event("log", "info", &format!("line {}", i)))
                .unwrap();
        }

        // First page
        let params = QueryParams {
            task_id: "pg1".to_string(),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 3,
            offset: 0,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 10);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].message, "line 0");

        // Second page
        let params = QueryParams {
            task_id: "pg1".to_string(),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 3,
            offset: 3,
        };
        let (events, _) = store.query_events(&params).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].message, "line 3");
    }

    #[test]
    fn query_events_empty_result() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("empty", "cmd", TaskStatus::Running)).unwrap();

        let params = QueryParams {
            task_id: "empty".to_string(),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 0);
        assert!(events.is_empty());
    }

    // ── Version cache tests ─────────────────────────────────────────────────

    #[test]
    fn version_cache_roundtrip() {
        let (store, _tmp) = test_store();
        store.cache_version("rustc", "1.75.0", 24).unwrap();

        let cached = store.get_cached_version("rustc").unwrap();
        assert_eq!(cached, Some("1.75.0".to_string()));
    }

    #[test]
    fn version_cache_miss() {
        let (store, _tmp) = test_store();
        let cached = store.get_cached_version("nonexistent").unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn version_cache_upsert() {
        let (store, _tmp) = test_store();
        store.cache_version("node", "18.0.0", 24).unwrap();
        store.cache_version("node", "20.0.0", 24).unwrap();

        let cached = store.get_cached_version("node").unwrap();
        assert_eq!(cached, Some("20.0.0".to_string()));
    }

    // ── Prune tests ─────────────────────────────────────────────────────────

    fn insert_tasks_with_offset(store: &Store, count: usize) {
        let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        for i in 0..count {
            let mut task = make_task(&format!("prune-{:03}", i), "cmd", TaskStatus::Completed);
            task.started_at = (base + chrono::Duration::days(i as i64)).to_rfc3339();
            store.insert_task(&task).unwrap();
            store
                .insert_event(&format!("prune-{:03}", i), 1, &make_event("log", "info", "msg"))
                .unwrap();
        }
    }

    #[test]
    fn prune_keep_basic() {
        let (store, _tmp) = test_store();
        insert_tasks_with_offset(&store, 10);

        let (dt, de) = store.prune_keep(3).unwrap();
        // Keep 3 most recent → cutoff is 3rd (prune-007), delete < prune-007 = 7 tasks
        assert_eq!(dt, 7);
        assert_eq!(de, 7);

        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn prune_keep_more_than_exists() {
        let (store, _tmp) = test_store();
        insert_tasks_with_offset(&store, 3);

        let (dt, de) = store.prune_keep(10).unwrap();
        assert_eq!(dt, 0);
        assert_eq!(de, 0);

        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn prune_keep_empty() {
        let (store, _tmp) = test_store();
        let (dt, de) = store.prune_keep(5).unwrap();
        assert_eq!(dt, 0);
        assert_eq!(de, 0);
    }

    #[test]
    fn prune_keep_zero_deletes_all() {
        let (store, _tmp) = test_store();
        insert_tasks_with_offset(&store, 5);

        let (dt, de) = store.prune_keep(0).unwrap();
        assert_eq!(dt, 5);
        assert_eq!(de, 5);

        let remaining = store.list_tasks(None, 100).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn prune_older_than_deletes_old() {
        let (store, _tmp) = test_store();

        // Insert a task from 60 days ago
        let old_date = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        let mut old_task = make_task("old-1", "old cmd", TaskStatus::Completed);
        old_task.started_at = old_date;
        store.insert_task(&old_task).unwrap();
        store.insert_event("old-1", 1, &make_event("log", "info", "old")).unwrap();

        // Insert a task from today
        store.insert_task(&make_task("new-1", "new cmd", TaskStatus::Running)).unwrap();
        store.insert_event("new-1", 1, &make_event("log", "info", "new")).unwrap();

        let (dt, de) = store.prune_older_than(30).unwrap();
        assert_eq!(dt, 1);
        assert_eq!(de, 1);

        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].task_id, "new-1");
    }

    #[test]
    fn prune_older_than_nothing_to_delete() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("recent", "cmd", TaskStatus::Running)).unwrap();

        let (dt, de) = store.prune_older_than(30).unwrap();
        assert_eq!(dt, 0);
        assert_eq!(de, 0);
    }

    // ── Raw output tests ────────────────────────────────────────────────────

    #[test]
    fn update_and_retrieve_raw_output() {
        let (store, _tmp) = test_store();
        let task = make_task("t1", "cargo test", TaskStatus::Completed);
        store.insert_task(&task).unwrap();
        store.update_task_raw_output("t1", "line1\nline2\nline3").unwrap();
        let raw = store.get_task_raw_output("t1").unwrap();
        assert_eq!(raw.as_deref(), Some("line1\nline2\nline3"));
    }

    #[test]
    fn raw_output_none_for_missing() {
        let (store, _tmp) = test_store();
        let raw = store.get_task_raw_output("nonexistent").unwrap();
        assert!(raw.is_none());
    }

    #[test]
    fn raw_output_overwrite() {
        let (store, _tmp) = test_store();
        let task = make_task("t2", "echo test", TaskStatus::Running);
        store.insert_task(&task).unwrap();
        store.update_task_raw_output("t2", "first version").unwrap();
        store.update_task_raw_output("t2", "second version").unwrap();
        let raw = store.get_task_raw_output("t2").unwrap();
        assert_eq!(raw.as_deref(), Some("second version"));
    }
}
