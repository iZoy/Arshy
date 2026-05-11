use arshy_lib::Result;

impl super::Store {
    /// Create all tables and indexes if they don't exist.
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

            CREATE INDEX IF NOT EXISTS idx_events_task ON events(task_id, seq);
            CREATE INDEX IF NOT EXISTS idx_events_type_sev ON events(task_id, type, severity);
            "
        )?;
        Ok(())
    }
}
