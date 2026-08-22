//! JSONL file-based storage — tasks and events.

mod events;
pub mod prune;
mod schema;
mod tasks;

use crate::ipc::TaskStatus;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Per-task metrics tracking parser pipeline throughput and enrichment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskMetrics {
    pub raw_output_bytes: u64,
    pub structured_events_bytes: u64,
    pub agent_visible_events: u64,
    pub agent_skipped_events: u64,
    pub locations_extracted: u64,
    pub codes_extracted: u64,
    pub contexts_enriched: u64,
    pub hints_attached: u64,
    pub pairs_merged: u64,
    pub agent_delivered_bytes: u64,
}

/// Internal task record extending the public Task with storage-only fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct TaskRecord {
    #[serde(flatten)]
    task: crate::ipc::Task,
    raw_output: Option<String>,
    dedup_collapsed: u64,
    correlated_errors: u64,
    metrics: TaskMetrics,
    /// Whether events have been enriched with context + hints.
    #[serde(default)]
    enriched: bool,
    /// Detected tool name for async enrichment (e.g., "cargo", "tsc").
    /// Stored so async enrichment can look up HintDb hints.
    #[serde(default)]
    detected_tool: Option<String>,
}

impl Default for TaskRecord {
    fn default() -> Self {
        Self {
            task: crate::ipc::Task {
                task_id: String::new(),
                command: String::new(),
                cwd: None,
                status: TaskStatus::Running,
                exit_code: None,
                pid: None,
                parser_name: None,
                started_at: String::new(),
                finished_at: None,
                duration_ms: None,
                events_count: 0,
                error_count: 0,
                purpose: None,
                carrier: None,
            },
            raw_output: None,
            dedup_collapsed: 0,
            correlated_errors: 0,
            metrics: TaskMetrics::default(),
            enriched: false,
            detected_tool: None,
        }
    }
}

/// Thread-safe JSONL file-based store.
///
/// Scale trigger (decision 4): JSONL is intentional — append-only, zero
/// deps, debuggable. If tasks exceed ~10k or cross-task query latency
/// exceeds ~200ms, add a derived index (e.g. file → task_id) before
/// considering a DB migration; no second reversal without measurement.
pub struct Store {
    dir: PathBuf,
    tasks: Mutex<HashMap<String, TaskRecord>>,
    dirty: AtomicBool,
    /// Epoch seconds of the most recent task activity. Initialised at open
    /// time so a never-used daemon still idle-exits after the timeout, and
    /// refreshed on every execution (short and long paths) so a daemon that
    /// only served short commands is not treated as idle forever.
    last_activity: AtomicI64,
    /// Notified whenever `mark_dirty` is called, so the background flush
    /// task can sleep indefinitely while idle instead of polling on a fixed
    /// interval (a 1 Hz poll would needlessly wake the CPU and hurt laptop
    /// battery life). A 30 s safety fallback guards against missed signals.
    notify: Arc<Notify>,
}

impl Store {
    /// Open (or create) the JSONL store directory at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let store_dir = path.to_path_buf();

        std::fs::create_dir_all(&store_dir)?;

        // Ensure raw output directory exists
        std::fs::create_dir_all(store_dir.join("raw"))?;

        let tasks_map = load_tasks_from_disk(&store_dir)?;

        Ok(Self {
            dir: store_dir,
            tasks: Mutex::new(tasks_map),
            dirty: AtomicBool::new(false),
            last_activity: AtomicI64::new(chrono::Utc::now().timestamp()),
            notify: Arc::new(Notify::new()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TaskRecord>> {
        self.tasks.lock().expect("store mutex poisoned")
    }

    /// Write the full tasks HashMap to tasks.jsonl atomically.
    fn persist_tasks(&self) -> Result<()> {
        let tasks = self.tasks.lock().unwrap();
        let path = self.dir.join("tasks.jsonl");
        let tmp = self.dir.join("tasks.jsonl.tmp");
        let mut file = std::fs::File::create(&tmp)?;
        for task in tasks.values() {
            serde_json::to_writer(&mut file, task)?;
            file.write_all(b"\n")?;
        }
        file.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        self.dirty.store(false, Ordering::Release);
        Ok(())
    }

    /// Mark the store as having unsaved changes and wake the flush task.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
        self.notify.notify_one();
    }

    /// Seconds since the most recent task activity, or `None` while a task is
    /// still running (the system is clearly not idle).
    ///
    /// Used by the idle-exit watchdog in the daemon accept loop: once this
    /// exceeds `daemon.idle_timeout_secs`, the daemon self-exits. A live
    /// task short-circuits to `None`. Activity covers both execution paths:
    /// long tasks mark on start and completion, short commands mark on start
    /// — so a daemon serving only short commands (which never touch the
    /// store) still idle-exits instead of lingering forever.
    pub fn idle_since_secs(&self) -> Result<Option<u64>> {
        let tasks = self.lock();
        for record in tasks.values() {
            if matches!(record.task.status, crate::ipc::TaskStatus::Running) {
                return Ok(None);
            }
        }
        let last = self.last_activity.load(Ordering::Relaxed);
        let secs = (chrono::Utc::now().timestamp() - last).max(0) as u64;
        Ok(Some(secs))
    }

    /// Record that a task ran (short or long path). Keeps the idle watchdog's
    /// "time since last activity" fresh for commands that skip the store.
    pub fn mark_activity(&self) {
        self.last_activity.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
    }

    /// Flush pending changes to disk if dirty. Returns Ok(()) even if not dirty.
    pub fn flush(&self) -> Result<()> {
        if self.dirty.load(Ordering::Acquire) {
            self.persist_tasks()?;
        }
        Ok(())
    }

    /// Spawn a background task that flushes dirty state to disk.
    ///
    /// Battery-friendly: the task sleeps until `mark_dirty` signals it (or a
    /// 30 s safety fallback elapses), so an idle daemon performs **zero**
    /// periodic wake-ups. Previously this polled every 1 s, which needlessly
    /// woke the CPU even when nothing was dirty. Call this once after the
    /// Store is wrapped in `Arc`.
    pub fn start_flush_task(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let store = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                // Wake only when signalled (dirty) or every 30 s as a guard.
                tokio::select! {
                    _ = store.notify.notified() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
                }
                if store.dirty.load(Ordering::Acquire) {
                    // persist_tasks acquires the mutex internally, safe to call
                    if let Err(e) = store.persist_tasks() {
                        tracing::warn!("background flush failed: {}", e);
                    }
                }
            }
        })
    }

    /// Append a single event JSON line to the per-task file.
    fn append_event_line(&self, task_id: &str, line: &str) -> Result<()> {
        let events_dir = self.dir.join("events");
        std::fs::create_dir_all(&events_dir)?;
        let path = events_dir.join(format!("{}.jsonl", task_id));
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }

    /// Write raw output to a per-task file under `<store_dir>/raw/<task_id>.txt`.
    fn write_raw_output(&self, task_id: &str, raw_output: &str) -> Result<()> {
        let raw_dir = self.dir.join("raw");
        std::fs::create_dir_all(&raw_dir)?;
        let path = raw_dir.join(format!("{}.txt", task_id));
        let tmp = raw_dir.join(format!("{}.txt.tmp", task_id));
        std::fs::write(&tmp, raw_output)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Read raw output from the per-task file, if it exists.
    fn read_raw_output(&self, task_id: &str) -> Result<Option<String>> {
        let path = self.dir.join("raw").join(format!("{}.txt", task_id));
        if path.exists() {
            Ok(Some(std::fs::read_to_string(&path)?))
        } else {
            Ok(None)
        }
    }

    /// Run `integrity_check` — verify tasks.jsonl is parseable.
    pub fn integrity_check(&self) -> Result<String> {
        let tasks = self.lock();
        // If we loaded successfully, the data is intact
        // Verify that the events directory is accessible
        let events_dir = self.dir.join("events");
        if events_dir.exists() {
            // Try to read each event file
            for entry in std::fs::read_dir(&events_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    let content = std::fs::read_to_string(&path)?;
                    for line in content.lines() {
                        if !line.trim().is_empty() {
                            let _: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                                crate::ArshyError::Other(format!(
                                    "corrupt event in {}: {}",
                                    path.display(),
                                    e
                                ))
                            })?;
                        }
                    }
                }
            }
        }
        drop(tasks);
        Ok("ok".to_string())
    }

    /// Return the on-disk store directory.
    pub fn store_dir(&self) -> &Path {
        &self.dir
    }
}

fn load_tasks_from_disk(dir: &Path) -> Result<HashMap<String, TaskRecord>> {
    let path = dir.join("tasks.jsonl");
    let mut map = HashMap::new();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        for (lineno, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<TaskRecord>(line) {
                Ok(mut record) => {
                    // Migrate inline raw_output to file-based storage if present
                    if let Some(ref raw) = record.raw_output {
                        if !raw.is_empty() {
                            let raw_dir = dir.join("raw");
                            let _ = std::fs::create_dir_all(&raw_dir);
                            let path = raw_dir.join(format!("{}.txt", record.task.task_id));
                            if !path.exists() {
                                let _ = std::fs::write(&path, raw);
                            }
                        }
                    }
                    record.raw_output = None;
                    map.insert(record.task.task_id.clone(), record);
                }
                Err(e) => {
                    tracing::warn!("tasks.jsonl line {} corrupt, skipping: {}", lineno + 1, e);
                }
            }
        }
    }
    Ok(map)
}

// ── Store Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{EventLocation, QueryParams, Task, TaskEvent, TaskStatus};
    use tempfile::TempDir;

    /// Create a temporary store with initialized schema.
    fn test_store() -> (Store, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
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
            purpose: None,
            carrier: None,
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

    // ── Idle watchdog tests ────────────────────────────────────────────────

    #[test]
    fn idle_since_secs_empty_store_reports_idle() {
        let (store, _tmp) = test_store();
        // An empty store (e.g. a daemon that only served short commands,
        // which never touch the store) must report an idle time — previously
        // it returned None and the daemon never idle-exited.
        assert!(store.idle_since_secs().unwrap().is_some());
    }

    #[test]
    fn idle_since_secs_running_task_returns_none() {
        let (store, _tmp) = test_store();
        let task = make_task("t1", "sleep 1", TaskStatus::Running);
        store.insert_task(&task).unwrap();
        assert!(store.idle_since_secs().unwrap().is_none());
    }

    #[test]
    fn mark_activity_refreshes_idle_time() {
        let (store, _tmp) = test_store();
        store.mark_activity();
        let secs = store.idle_since_secs().unwrap().unwrap();
        assert!(secs <= 1, "idle time should be ~0 right after activity, got {}", secs);
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
        // Second insert with same ID should error (duplicate)
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
            task_id: Some("evt-1".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
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
            task_id: Some("loc-1".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
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
            task_id: Some("f1".to_string()),
            event_type: Some("diagnostic".to_string()),
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
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
            task_id: Some("f2".to_string()),
            event_type: None,
            severity: Some("error".to_string()),
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
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
            task_id: Some("f3".to_string()),
            event_type: Some("diagnostic".to_string()),
            severity: Some("error".to_string()),
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
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
            task_id: Some("pg1".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 3,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 10);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].message, "line 0");

        // Second page
        let params = QueryParams {
            task_id: Some("pg1".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 3,
            offset: 3,
            include_logs: true,
        };
        let (events, _) = store.query_events(&params).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].message, "line 3");
    }

    #[test]
    fn search_events_across_tasks() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("s1", "cmd", TaskStatus::Completed)).unwrap();
        store.insert_task(&make_task("s2", "cmd", TaskStatus::Completed)).unwrap();
        let mut e1 = make_event("diagnostic", "error", "TS2345 in alpha");
        e1.code = Some("TS2345".to_string());
        e1.location = Some(EventLocation { file: "src/a.ts".to_string(), line: 1, column: None });
        let mut e2 = make_event("diagnostic", "error", "E0308 in beta");
        e2.code = Some("E0308".to_string());
        e2.location = Some(EventLocation { file: "src/b.rs".to_string(), line: 2, column: None });
        let e3 = make_event("log", "info", "noise line");
        store.insert_event("s1", 1, &e1).unwrap();
        store.insert_event("s1", 2, &e3).unwrap();
        store.insert_event("s2", 1, &e2).unwrap();

        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: false,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 2, "log events excluded by default");
        assert_eq!(events.len(), 2);
        let ids: std::collections::HashSet<&str> =
            events.iter().map(|e| e["task_id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), 2, "events must carry their owning task_id");

        // Filter by error code across tasks.
        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: Some("TS2345".to_string()),
            file: None,
            limit: 100,
            offset: 0,
            include_logs: false,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0]["code"].as_str(), Some("TS2345"));
        assert_eq!(events[0]["task_id"].as_str(), Some("s1"));

        // Filter by file path substring across tasks.
        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: Some("src/b.rs".to_string()),
            limit: 100,
            offset: 0,
            include_logs: false,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0]["task_id"].as_str(), Some("s2"));
    }

    #[test]
    fn search_events_respects_limit_and_include_logs() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("l1", "cmd", TaskStatus::Completed)).unwrap();
        store.insert_task(&make_task("l2", "cmd", TaskStatus::Completed)).unwrap();
        for i in 0..3 {
            store.insert_event("l1", i, &make_event("log", "info", &format!("line {i}"))).unwrap();
        }
        store.insert_event("l2", 1, &make_event("diagnostic", "error", "boom")).unwrap();

        // include_logs=false → only the diagnostic across both tasks.
        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 10,
            offset: 0,
            include_logs: false,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0]["type"].as_str(), Some("diagnostic"));

        // include_logs=true + limit=2 → pagination over the cross-task set.
        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 2,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 4);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn query_events_empty_result() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("empty", "cmd", TaskStatus::Running)).unwrap();

        let params = QueryParams {
            task_id: Some("empty".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 0);
        assert!(events.is_empty());
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

    // ── Enriched flag tests ───────────────────────────────────────────────────

    #[test]
    fn is_enriched_defaults_false() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("enr-1", "cmd", TaskStatus::Running)).unwrap();
        assert!(!store.is_enriched("enr-1"));
    }

    #[test]
    fn mark_enriched_sets_flag() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("enr-2", "cmd", TaskStatus::Running)).unwrap();
        assert!(!store.is_enriched("enr-2"));
        store.mark_enriched("enr-2").unwrap();
        assert!(store.is_enriched("enr-2"));
    }

    #[test]
    fn mark_enriched_nonexistent_is_noop() {
        let (store, _tmp) = test_store();
        // Should not error
        store.mark_enriched("ghost").unwrap();
    }

    #[test]
    fn mark_enriched_persists_across_reload() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        store.initialize_schema().unwrap();

        store.insert_task(&make_task("enr-3", "cmd", TaskStatus::Running)).unwrap();
        store.mark_enriched("enr-3").unwrap();
        // Flush to disk — no background task in tests
        store.flush().unwrap();

        // Reload store from disk
        let store2 = Store::open(&db_path).unwrap();
        assert!(store2.is_enriched("enr-3"));
    }

    #[test]
    fn is_enriched_unknown_task_returns_false() {
        let (store, _tmp) = test_store();
        assert!(!store.is_enriched("nonexistent"));
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
