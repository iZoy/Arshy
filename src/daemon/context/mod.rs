//! Error context extraction — reads source files around error locations.

pub mod git_correlator;

use arshy_lib::ipc::{EventContext, EventLocation, TaskEvent};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Extract ±3 lines of source context around `line` in `file`.
/// Returns None if the file can't be read or the line is out of range.
#[allow(dead_code)] // sync version; async variant used in production
pub fn extract_context(file: &str, line: u64) -> Option<EventContext> {
    let content = std::fs::read_to_string(file).ok()?;
    parse_context(&content, line)
}

/// Async version — reads the file using tokio::fs to avoid blocking.
pub async fn extract_context_async(file: &str, line: u64) -> Option<EventContext> {
    let content = tokio::fs::read_to_string(file).await.ok()?;
    parse_context(&content, line)
}

/// Parse context from file content string. Shared between sync and async paths.
fn parse_context(content: &str, line: u64) -> Option<EventContext> {
    let lines: Vec<&str> = content.lines().collect();
    let idx = (line as usize).saturating_sub(1);

    if idx >= lines.len() {
        return None;
    }

    let before: Vec<String> =
        lines[idx.saturating_sub(3)..idx].iter().map(|s| s.to_string()).collect();
    let after: Vec<String> =
        lines[idx + 1..(idx + 4).min(lines.len())].iter().map(|s| s.to_string()).collect();

    Some(EventContext { before, line: lines[idx].to_string(), after })
}

/// Enriches error/warning events with surrounding source context.
/// Caches file reads to avoid re-reading the same file for multiple errors.
#[allow(dead_code)] // used in integration (Task 3); tests exercise via direct calls
pub struct ContextEnricher {
    context_lines: usize,
    file_cache: HashMap<PathBuf, Vec<String>>,
}

#[allow(dead_code)] // used in integration (Task 3); tests exercise via direct calls
impl ContextEnricher {
    pub fn new(context_lines: usize) -> Self {
        Self { context_lines, file_cache: HashMap::new() }
    }

    /// Enrich error/warning events that have a file:line location.
    /// Populates EventContext with surrounding source lines.
    /// Skips events that already have context, aren't error/warning,
    /// or have no location.
    pub fn enrich(&mut self, events: &mut [TaskEvent], cwd: &Path) {
        for event in events.iter_mut() {
            if event.context.is_some() {
                continue;
            }
            if !matches!(event.severity.as_deref(), Some("error") | Some("warning")) {
                continue;
            }
            let loc = match &event.location {
                Some(loc) => loc,
                None => continue,
            };
            if let Some(ctx) = self.read_context(cwd, loc) {
                event.context = Some(ctx);
            }
        }
    }

    fn read_context(&mut self, cwd: &Path, loc: &EventLocation) -> Option<EventContext> {
        let path = cwd.join(&loc.file);
        let lines = self
            .file_cache
            .entry(path.clone())
            .or_insert_with(|| {
                std::fs::read_to_string(&path)
                    .ok()
                    .map(|c| c.lines().map(String::from).collect())
                    .unwrap_or_default()
            })
            .clone();
        if lines.is_empty() {
            return None;
        }
        let idx = loc.line.checked_sub(1)? as usize;
        if idx >= lines.len() {
            return None;
        }
        let before = lines[idx.saturating_sub(self.context_lines)..idx].to_vec();
        let line = lines[idx].clone();
        let after = lines[idx + 1..(idx + 1 + self.context_lines).min(lines.len())].to_vec();
        Some(EventContext { before, line, after })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_middle_line() {
        let ctx = extract_context("Cargo.toml", 1).unwrap();
        assert!(!ctx.line.is_empty());
    }

    #[test]
    fn out_of_range_returns_none() {
        assert!(extract_context("Cargo.toml", 99999).is_none());
    }

    #[test]
    fn missing_file_returns_none() {
        assert!(extract_context("/nonexistent/file.txt", 1).is_none());
    }

    #[tokio::test]
    async fn async_extract_context() {
        let ctx = extract_context_async("Cargo.toml", 1).await.unwrap();
        assert!(!ctx.line.is_empty());
    }

    #[tokio::test]
    async fn async_missing_file_returns_none() {
        assert!(extract_context_async("/nonexistent/file.txt", 1).await.is_none());
    }

    #[test]
    fn enricher_populates_context_for_errors() {
        let mut enricher = ContextEnricher::new(3);
        let mut events = vec![TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "test error".into(),
            location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
            context: None,
        }];
        enricher.enrich(&mut events, std::path::Path::new("."));
        assert!(events[0].context.is_some());
        assert!(!events[0].context.as_ref().unwrap().line.is_empty());
    }

    #[test]
    fn enricher_skips_info_events() {
        let mut enricher = ContextEnricher::new(3);
        let mut events = vec![TaskEvent {
            seq: 0,
            event_type: "log".into(),
            severity: Some("info".into()),
            code: None,
            message: "info message".into(),
            location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
            context: None,
        }];
        enricher.enrich(&mut events, std::path::Path::new("."));
        assert!(events[0].context.is_none());
    }

    #[test]
    fn enricher_skips_events_without_location() {
        let mut enricher = ContextEnricher::new(3);
        let mut events = vec![TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "no location".into(),
            location: None,
            context: None,
        }];
        enricher.enrich(&mut events, std::path::Path::new("."));
        assert!(events[0].context.is_none());
    }

    #[test]
    fn enricher_skips_existing_context() {
        let mut enricher = ContextEnricher::new(3);
        let mut events = vec![TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "already has context".into(),
            location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
            context: Some(EventContext { before: vec![], line: "existing".into(), after: vec![] }),
        }];
        enricher.enrich(&mut events, std::path::Path::new("."));
        assert_eq!(events[0].context.as_ref().unwrap().line, "existing");
    }

    #[test]
    fn enricher_handles_missing_file() {
        let mut enricher = ContextEnricher::new(3);
        let mut events = vec![TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "missing file".into(),
            location: Some(EventLocation {
                file: "/nonexistent/file.rs".into(),
                line: 1,
                column: None,
            }),
            context: None,
        }];
        enricher.enrich(&mut events, std::path::Path::new("."));
        assert!(events[0].context.is_none());
    }

    #[test]
    fn enricher_caches_file_reads() {
        let mut enricher = ContextEnricher::new(3);
        let mut events = vec![
            TaskEvent {
                seq: 0,
                event_type: "diagnostic".into(),
                severity: Some("error".into()),
                code: None,
                message: "error 1".into(),
                location: Some(EventLocation { file: "Cargo.toml".into(), line: 1, column: None }),
                context: None,
            },
            TaskEvent {
                seq: 1,
                event_type: "diagnostic".into(),
                severity: Some("error".into()),
                code: None,
                message: "error 2".into(),
                location: Some(EventLocation { file: "Cargo.toml".into(), line: 2, column: None }),
                context: None,
            },
        ];
        enricher.enrich(&mut events, std::path::Path::new("."));
        assert!(events[0].context.is_some());
        assert!(events[1].context.is_some());
    }
}
