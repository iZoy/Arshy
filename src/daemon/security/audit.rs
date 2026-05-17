//! Audit log — append-only JSON-lines log of all command executions.

use arshy_lib::Result;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Append-only audit log writer.
pub struct AuditLog {
    /// Stored for future audit log rotation and tooling (path inspection).
    #[allow(dead_code)]
    path: PathBuf,
    file: Mutex<std::fs::File>,
}

/// A single audit log entry.
#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub task_id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub blocked: bool,
    pub reason: Option<String>,
}

impl AuditLog {
    /// Open (or create) an append-only audit log at `path`.
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { path: path.to_path_buf(), file: Mutex::new(file) })
    }

    /// Append a single entry as a JSON line.
    pub fn log(&self, entry: &AuditEntry) -> Result<()> {
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        let mut file = self.file.lock().map_err(|_| {
            arshy_lib::ArshyError::Other("audit log mutex poisoned".into())
        })?;
        file.write_all(line.as_bytes())
            .map_err(arshy_lib::ArshyError::Io)
    }

    /// Path to the audit log file. Reserved for audit log tooling.
    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_entry(task_id: &str, command: &str) -> AuditEntry {
        AuditEntry {
            timestamp: chrono::Utc::now(),
            task_id: task_id.to_string(),
            command: command.to_string(),
            cwd: Some("/tmp".to_string()),
            exit_code: Some(0),
            blocked: false,
            reason: None,
        }
    }

    #[test]
    fn log_creates_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("audit.log");
        let log = AuditLog::new(&path).unwrap();

        log.log(&test_entry("t1", "echo hello")).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn entries_are_json_lines() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("audit.log");
        let log = AuditLog::new(&path).unwrap();

        log.log(&test_entry("t1", "echo a")).unwrap();
        log.log(&test_entry("t2", "echo b")).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 2);

        // Each line should be valid JSON
        let entry1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry1["task_id"], "t1");
        assert_eq!(entry1["command"], "echo a");

        let entry2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(entry2["task_id"], "t2");
        assert_eq!(entry2["command"], "echo b");
    }

    #[test]
    fn append_only_preserves_existing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("audit.log");

        // Write first entry
        let log1 = AuditLog::new(&path).unwrap();
        log1.log(&test_entry("t1", "first")).unwrap();
        drop(log1);

        // Reopen and write second entry
        let log2 = AuditLog::new(&path).unwrap();
        log2.log(&test_entry("t2", "second")).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 2, "append-only should preserve both entries");
    }

    #[test]
    fn blocked_entry_has_reason() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("audit.log");
        let log = AuditLog::new(&path).unwrap();

        let entry = AuditEntry {
            timestamp: chrono::Utc::now(),
            task_id: "t-blocked".into(),
            command: "rm -rf /".into(),
            cwd: None,
            exit_code: None,
            blocked: true,
            reason: Some("matches pattern 'rm\\s+-rf\\s+/'".into()),
        };
        log.log(&entry).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["blocked"], true);
        assert!(parsed["reason"].as_str().unwrap().contains("rm"));
    }

    #[test]
    fn creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("deep").join("nested").join("audit.log");
        let log = AuditLog::new(&path).unwrap();
        log.log(&test_entry("t1", "test")).unwrap();
        assert!(path.exists());
    }
}
