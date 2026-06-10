//! Git diff correlation — identifies which error files were recently changed.

use arshy_lib::ipc::TaskEvent;
use std::path::Path;

/// Correlates error events with recently changed files from git diff.
pub struct GitCorrelation {
    changed_files: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)] // used in integration (Task 3); tests exercise via direct calls
pub struct CorrelatedError {
    pub file: String,
    pub recently_changed: bool,
}

impl GitCorrelation {
    /// Detect recently changed files from `git diff --name-only HEAD~1`.
    pub fn detect(cwd: Option<&Path>) -> Option<Self> {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["diff", "--name-only", "HEAD~1"]);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<String> =
            stdout.lines().filter(|l| !l.is_empty()).map(String::from).collect();
        if files.is_empty() {
            return None;
        }
        Some(Self { changed_files: files })
    }

    /// Correlate error events with changed files.
    #[allow(dead_code)] // used in integration (Task 3); tests exercise via direct calls
    pub fn correlate(&self, events: &[TaskEvent]) -> Vec<CorrelatedError> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for event in events {
            if event.severity.as_deref() != Some("error") {
                continue;
            }
            let file = match &event.location {
                Some(loc) => &loc.file,
                None => continue,
            };
            if seen.contains(file) {
                continue;
            }
            seen.insert(file.clone());
            results.push(CorrelatedError {
                file: file.clone(),
                recently_changed: self.changed_files.iter().any(|f| f == file),
            });
        }
        results
    }

    /// Get the list of changed files.
    pub fn changed_files(&self) -> &[String] {
        &self.changed_files
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::ipc::EventLocation;

    fn make_error(file: &str, line: u64) -> TaskEvent {
        TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "test".into(),
            location: Some(EventLocation { file: file.into(), line, column: None }),
            context: None,
            hint: None,
        }
    }

    #[test]
    fn correlate_matches_changed_files() {
        let gc = GitCorrelation { changed_files: vec!["src/main.rs".into(), "Cargo.toml".into()] };
        let events = vec![make_error("src/main.rs", 42), make_error("src/lib.rs", 10)];
        let results = gc.correlate(&events);
        assert_eq!(results.len(), 2);
        assert!(results[0].recently_changed);
        assert!(!results[1].recently_changed);
    }

    #[test]
    fn correlate_deduplicates_files() {
        let gc = GitCorrelation { changed_files: vec!["src/main.rs".into()] };
        let events = vec![make_error("src/main.rs", 10), make_error("src/main.rs", 20)];
        let results = gc.correlate(&events);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn correlate_skips_non_errors() {
        let gc = GitCorrelation { changed_files: vec!["src/main.rs".into()] };
        let mut event = make_error("src/main.rs", 10);
        event.severity = Some("warning".into());
        let results = gc.correlate(&[event]);
        assert!(results.is_empty());
    }

    #[test]
    fn changed_files_returns_list() {
        let gc = GitCorrelation { changed_files: vec!["a.rs".into(), "b.rs".into()] };
        assert_eq!(gc.changed_files().len(), 2);
    }
}
