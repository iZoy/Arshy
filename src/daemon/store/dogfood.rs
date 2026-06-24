//! Dogfood run persistence — tracks results of background dogfood checks.

use arshy_lib::Result;
use serde::{Deserialize, Serialize};
use std::io::Write as _;

/// A single dogfood run result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DogfoodRun {
    pub timestamp: String,
    pub pass: u32,
    pub fail: u32,
    pub total: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_excerpt: Option<String>,
}

impl super::Store {
    /// Append a dogfood run result to dogfood.jsonl.
    pub fn insert_dogfood_run(&self, run: &DogfoodRun) -> Result<()> {
        let path = self.dir.join("dogfood.jsonl");
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        serde_json::to_writer(&mut file, run)?;
        file.write_all(b"\n")?;
        Ok(())
    }

    /// Read the last N dogfood runs (most recent first).
    #[allow(dead_code)] // Used by IPC handler for dogfood queries
    pub fn list_dogfood_runs(&self, limit: usize) -> Result<Vec<DogfoodRun>> {
        let path = self.dir.join("dogfood.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut runs: Vec<DogfoodRun> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        runs.reverse();
        runs.truncate(limit);
        Ok(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_store() -> (Arc<super::super::Store>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = Arc::new(super::super::Store::open(tmp.path(), false).unwrap());
        store.initialize_schema().unwrap();
        (store, tmp)
    }

    #[test]
    fn insert_and_list_dogfood_run() {
        let (store, _tmp) = test_store();
        let run = DogfoodRun {
            timestamp: "2026-06-25T10:00:00Z".to_string(),
            pass: 21,
            fail: 0,
            total: 21,
            log_excerpt: None,
        };
        store.insert_dogfood_run(&run).unwrap();
        let runs = store.list_dogfood_runs(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].pass, 21);
        assert_eq!(runs[0].total, 21);
    }

    #[test]
    fn list_returns_most_recent_first() {
        let (store, _tmp) = test_store();
        for i in 0..3 {
            let run = DogfoodRun {
                timestamp: format!("2026-06-25T{:02}:00:00Z", i),
                pass: 20 + i,
                fail: 1 - i.min(1),
                total: 21,
                log_excerpt: None,
            };
            store.insert_dogfood_run(&run).unwrap();
        }
        let runs = store.list_dogfood_runs(2).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].pass, 22); // most recent
        assert_eq!(runs[1].pass, 21);
    }

    #[test]
    fn list_empty_when_no_file() {
        let (store, _tmp) = test_store();
        let runs = store.list_dogfood_runs(10).unwrap();
        assert!(runs.is_empty());
    }
}
