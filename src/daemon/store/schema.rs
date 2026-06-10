use arshy_lib::Result;

impl super::Store {
    /// Create all tables and indexes if they don't exist, then run migrations.
    pub fn initialize_schema(&self) -> Result<()> {
        self.lock().execute_batch(
            "
            CREATE TABLE IF NOT EXISTS tasks (
                task_id      TEXT PRIMARY KEY,
                command      TEXT NOT NULL,
              cwd          TEXT,
                status       TEXT NOT NULL,
                exit_code    INTEGER,
                pid          INTEGER,
                parser_name  TEXT,
                started_at   TEXT NOT NULL,
                finished_at  TEXT,
                duration_ms  INTEGER,
                events_count INTEGER DEFAULT 0,
                error_count  INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS events (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id      TEXT NOT NULL REFERENCES tasks(task_id),
                seq          INTEGER NOT NULL,
                type         TEXT NOT NULL,
                severity     TEXT,
                code         TEXT,
                message      TEXT NOT NULL,
                payload      TEXT NOT NULL,
                created_at   TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tool_versions (
                tool_name   TEXT PRIMARY KEY,
                version     TEXT NOT NULL,
                detected_at TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS parser_registry (
                parser_name TEXT PRIMARY KEY,
                tool_name   TEXT NOT NULL,
                min_version TEXT,
                max_version TEXT,
                parser_type TEXT NOT NULL,
                source      TEXT NOT NULL,
                loaded_at   TEXT
            );

            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_events_task ON events(task_id, seq);
            CREATE INDEX IF NOT EXISTS idx_events_type_sev ON events(task_id, type, severity);
            ",
        )?;

        self.run_migrations()?;
        Ok(())
    }

    /// Apply any pending migrations.
    fn run_migrations(&self) -> Result<()> {
        let conn = self.lock();
        let current: i64 = conn
            .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |r| r.get(0))
            .unwrap_or(0);

        if current < 2 {
            // v2: add index on tasks.started_at for efficient pruning
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_tasks_started ON tasks(started_at);
                 INSERT INTO schema_version (version) VALUES (2);",
            )?;
        }

        if current < 3 {
            // v3: add index on tasks.status for fast status-filtered listing
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
                 INSERT INTO schema_version (version) VALUES (3);",
            )?;
        }

        if current < 4 {
            // v4: add raw_output column for tee / failure recovery
            conn.execute_batch(
                "ALTER TABLE tasks ADD COLUMN raw_output TEXT;
                 INSERT INTO schema_version (version) VALUES (4);",
            )?;
        }

        Ok(())
    }

    /// Run a WAL checkpoint to reclaim space. No-op if WAL mode is not active.
    pub fn wal_checkpoint(&self) -> Result<()> {
        self.lock().execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    /// Return aggregate stats about tasks and events.
    pub fn get_stats(
        &self,
        db_path: Option<&std::path::Path>,
    ) -> Result<arshy_lib::ipc::StatsResponse> {
        use arshy_lib::ipc::StatusCounts;
        let conn = self.lock();

        let total_tasks: u64 =
            conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0)).unwrap_or(0);

        let mut counts = StatusCounts::default();
        let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
        let rows =
            stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)))?;
        for row in rows {
            let (status, count) = row?;
            // Status is stored as JSON-quoted string (e.g. "\"completed\"")
            let trimmed = status.trim_matches('"');
            match trimmed {
                "running" => counts.running = count,
                "completed" => counts.completed = count,
                "failed" => counts.failed = count,
                "killed" => counts.killed = count,
                "timeout" => counts.timeout = count,
                _ => {}
            }
        }

        let total_events: u64 =
            conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap_or(0);
        let total_errors: u64 = conn
            .query_row("SELECT COUNT(*) FROM events WHERE severity = 'error'", [], |r| r.get(0))
            .unwrap_or(0);

        let avg_duration: Option<f64> = conn
            .query_row(
                "SELECT AVG(duration_ms) FROM tasks WHERE duration_ms IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .ok();

        // Percentiles via ordered list
        let durations: Vec<u64> = conn
            .prepare(
                "SELECT duration_ms FROM tasks WHERE duration_ms IS NOT NULL ORDER BY duration_ms",
            )?
            .query_map([], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let p50 = percentile(&durations, 50);
        let p99 = percentile(&durations, 99);

        let failure_rate = if total_tasks > 0 {
            let failed_total = counts.failed + counts.timeout;
            Some(failed_total as f64 / total_tasks as f64)
        } else {
            None
        };

        let db_size = db_path.and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());

        let parser_coverage_pct: Option<f64> = conn
            .query_row(
                "SELECT CASE WHEN COUNT(*) = 0 THEN 0.0 \
                 ELSE CAST(COUNT(CASE WHEN type != 'log' THEN 1 END) AS REAL) \
                     * 100.0 / COUNT(*) END \
                 FROM events",
                [],
                |r| r.get::<_, f64>(0),
            )
            .ok();

        // Count events that received fix hints (hint field stored in payload JSON)
        let hints_attached: Option<u64> = conn
            .query_row("SELECT COUNT(*) FROM events WHERE payload LIKE '%\"hint\":%'", [], |r| {
                r.get::<_, i64>(0)
            })
            .ok()
            .map(|v| v as u64);

        // Count events enriched with source context (context field with before/after)
        let context_enriched: Option<u64> = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE payload LIKE '%\"context\":%' AND payload NOT LIKE '%\"context\":null%'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .ok()
            .map(|v| v as u64);

        Ok(arshy_lib::ipc::StatsResponse {
            total_tasks,
            by_status: counts,
            total_events,
            total_errors,
            avg_duration_ms: avg_duration,
            p50_duration_ms: p50,
            p99_duration_ms: p99,
            failure_rate,
            db_size_bytes: db_size,
            parser_coverage_pct,
            hints_attached,
            context_enriched,
        })
    }
}

fn percentile(sorted: &[u64], pct: u64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = (pct as usize * (sorted.len() - 1)) / 100;
    sorted.get(idx).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::ipc::{Task, TaskEvent, TaskStatus};
    use tempfile::TempDir;

    fn test_store() -> (super::super::Store, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("test.db");
        let store = super::super::Store::open(&db, false).unwrap();
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

    #[test]
    fn percentile_empty() {
        assert_eq!(percentile(&[], 50), None);
    }

    #[test]
    fn percentile_single() {
        assert_eq!(percentile(&[42], 50), Some(42));
        assert_eq!(percentile(&[42], 99), Some(42));
    }

    #[test]
    fn percentile_multiple() {
        let data = [10, 20, 30, 40, 50];
        assert_eq!(percentile(&data, 0), Some(10));
        assert_eq!(percentile(&data, 50), Some(30));
        assert_eq!(percentile(&data, 100), Some(50));
    }

    #[test]
    fn wal_checkpoint_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("test.db");
        let store = super::super::Store::open(&db, false).unwrap();
        store.initialize_schema().unwrap();
        // Should not error even without WAL mode
        store.wal_checkpoint().unwrap();
    }

    #[test]
    fn stats_empty_store() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("test.db");
        let store = super::super::Store::open(&db, false).unwrap();
        store.initialize_schema().unwrap();

        let stats = store.get_stats(Some(&db)).unwrap();
        assert_eq!(stats.total_tasks, 0);
        assert_eq!(stats.total_events, 0);
        assert!(stats.avg_duration_ms.is_none());
    }

    #[test]
    fn stats_intelligence_metrics() {
        let (store, _tmp) = test_store();
        let task = make_task("t1", "cargo test", TaskStatus::Failed);
        store.insert_task(&task).unwrap();
        // Insert an event with hint in payload
        let hint_event = TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: Some("E0308".into()),
            message: "mismatched types".into(),
            location: None,
            context: None,
            hint: Some(arshy_lib::ipc::EventHint {
                cause: "Type mismatch".into(),
                fix: Some("Use .into()".into()),
                retry: None,
            }),
        };
        store.insert_event("t1", 0, &hint_event).unwrap();
        store.insert_event("t1", 1, &make_event("log", "info", "line2")).unwrap();
        store.insert_event("t1", 2, &make_event("log", "info", "line3")).unwrap();

        let stats = store.get_stats(None).unwrap();
        assert!(stats.parser_coverage_pct.is_some());
        // 1 of 3 events is type != 'log'
        let coverage = stats.parser_coverage_pct.unwrap();
        assert!(coverage > 30.0 && coverage < 40.0); // ~33.3%
                                                     // 1 event has hint
        assert_eq!(stats.hints_attached, Some(1));
    }

    #[test]
    fn schema_migration_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("test.db");
        let store = super::super::Store::open(&db, false).unwrap();
        store.initialize_schema().unwrap();
        // Second call should not error
        store.initialize_schema().unwrap();
    }
}
