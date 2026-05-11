use arshy_lib::Result;

impl super::Store {
    /// Keep only the `keep` most recent tasks, delete the rest.
    /// Returns (tasks_deleted, events_deleted).
    pub fn prune_keep(&self, keep: usize) -> Result<(usize, usize)> {
        let conn = self.lock();
        // Find the task_id cutoff: the oldest of the keep most recent
        let cutoff: Option<String> = conn.query_row(
            "SELECT task_id FROM tasks ORDER BY started_at DESC LIMIT 1 OFFSET ?1",
            rusqlite::params![keep as i64],
            |r| r.get(0),
        ).ok();

        if cutoff.is_none() {
            return Ok((0, 0));
        }
        let cutoff = cutoff.unwrap();

        let de: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE task_id < ?1",
            rusqlite::params![cutoff],
            |r| r.get(0),
        )?;
        let dt: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE task_id < ?1",
            rusqlite::params![cutoff],
            |r| r.get(0),
        )?;

        conn.execute("DELETE FROM events WHERE task_id < ?1", rusqlite::params![cutoff])?;
        conn.execute("DELETE FROM tasks WHERE task_id < ?1", rusqlite::params![cutoff])?;

        Ok((dt as usize, de as usize))
    }

    /// Delete tasks older than `days` days. Returns (tasks_deleted, events_deleted).
    pub fn prune_older_than(&self, days: u32) -> Result<(usize, usize)> {
        let conn = self.lock();
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();

        let de: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE task_id IN (SELECT task_id FROM tasks WHERE started_at < ?1)",
            rusqlite::params![cutoff],
            |r| r.get(0),
        )?;
        let dt: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE started_at < ?1",
            rusqlite::params![cutoff],
            |r| r.get(0),
        )?;

        conn.execute(
            "DELETE FROM events WHERE task_id IN (SELECT task_id FROM tasks WHERE started_at < ?1)",
            rusqlite::params![cutoff],
        )?;
        conn.execute("DELETE FROM tasks WHERE started_at < ?1", rusqlite::params![cutoff])?;

        Ok((dt as usize, de as usize))
    }
}
