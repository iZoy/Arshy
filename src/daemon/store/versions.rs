use crate::Result;

impl super::Store {
    /// Get a cached tool version if still valid (not expired).
    pub fn get_cached_version(&self, tool_name: &str) -> Result<Option<String>> {
        let versions = self.versions.lock().unwrap();
        if let Some((version, expires_at)) = versions.get(tool_name) {
            if *expires_at > chrono::Utc::now() {
                return Ok(Some(version.clone()));
            }
        }
        Ok(None)
    }

    /// Cache a detected tool version (upsert) with a TTL in hours.
    pub fn cache_version(&self, tool_name: &str, version: &str, ttl_hours: u64) -> Result<()> {
        let mut versions = self.versions.lock().unwrap();
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(ttl_hours as i64);
        versions.insert(tool_name.to_string(), (version.to_string(), expires_at));
        drop(versions);
        self.persist_versions()
    }
}
