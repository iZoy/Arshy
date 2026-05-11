use arshy_lib::Result;

impl super::Store {
    /// Get a cached tool version if still valid (not expired).
    pub fn get_cached_version(&self, tool_name: &str) -> Result<Option<String>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT version FROM tool_versions WHERE tool_name=?1 AND expires_at > datetime('now')"
        )?;
        let mut rows = stmt.query_map(rusqlite::params![tool_name], |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(Ok(v)) => Ok(Some(v)),
            _ => Ok(None),
        }
    }

    /// Cache a detected tool version (upsert) with a TTL in hours.
    pub fn cache_version(&self, tool_name: &str, version: &str, ttl_hours: u64) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO tool_versions (tool_name, version, detected_at, expires_at)
             VALUES (?1, ?2, datetime('now'), datetime('now', '+' || ?3 || ' hours'))",
            rusqlite::params![tool_name, version, ttl_hours as i64],
        )?;
        Ok(())
    }
}
