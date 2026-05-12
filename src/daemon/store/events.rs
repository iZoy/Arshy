use arshy_lib::ipc::{QueryParams, TaskEvent};
use arshy_lib::Result;

impl super::Store {
    /// Insert a structured event for a task.
    pub fn insert_event(&self, task_id: &str, seq: u64, event: &TaskEvent) -> Result<()> {
        let conn = self.lock();
        let payload = serde_json::to_string(event)?;
        conn.execute(
            "INSERT INTO events (task_id, seq, type, severity, code, message, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                task_id, seq as i64,
                event.event_type, event.severity, event.code, event.message,
                payload, chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        // Bump counts
        conn.execute(
            "UPDATE tasks SET events_count = events_count + 1,
             error_count = error_count + CASE WHEN ?1 = 'error' THEN 1 ELSE 0 END
             WHERE task_id = ?2",
            rusqlite::params![event.severity, task_id],
        )?;
        Ok(())
    }

    /// Query events with optional filters. Returns (events, total_count).
    pub fn query_events(&self, params: &QueryParams) -> Result<(Vec<TaskEvent>, usize)> {
        let conn = self.lock();

        let mut conditions = vec!["task_id = ?1".to_string()];
        let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(params.task_id.clone())];

        if let Some(t) = &params.event_type {
            conditions.push(format!("type = ?{}", binds.len() + 1));
            binds.push(Box::new(t.clone()));
        }
        if let Some(s) = &params.severity {
            conditions.push(format!("severity = ?{}", binds.len() + 1));
            binds.push(Box::new(s.clone()));
        }
        if let Some(c) = &params.code {
            conditions.push(format!("code = ?{}", binds.len() + 1));
            binds.push(Box::new(c.clone()));
        }

        let where_clause = conditions.join(" AND ");

        // Total count
        let count_sql = format!("SELECT COUNT(*) FROM events WHERE {}", where_clause);
        let total: usize = {
            let mut stmt = conn.prepare(&count_sql)?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
            stmt.query_row(params_refs.as_slice(), |r| r.get::<_, i64>(0))
                .map(|v| v as usize)?
        };

        // Fetch page
        let query_sql = format!(
            "SELECT payload FROM events WHERE {} ORDER BY seq ASC LIMIT ?{} OFFSET ?{}",
            where_clause,
            binds.len() + 1,
            binds.len() + 2,
        );
        binds.push(Box::new(params.limit as i64));
        binds.push(Box::new(params.offset as i64));

        let mut stmt = conn.prepare(&query_sql)?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            let payload: String = row.get(0)?;
            Ok(payload)
        })?;

        let mut events = Vec::new();
        for payload in rows.flatten() {
            if let Ok(event) = serde_json::from_str::<TaskEvent>(&payload) {
                events.push(event);
            }
        }

        Ok((events, total))
    }
}
