//! Heuristic error filter — tier 4.5 in the parser pipeline.
//!
//! Catches error/warning keywords in lines that weren't matched by any
//! tool-specific parser. Extracts file:line:col when present.

use arshy_lib::ipc::{EventLocation, TaskEvent};
use regex::Regex;
use std::sync::LazyLock;

/// Try to match a line using heuristic error/warning detection.
/// Returns `Some(event)` if a keyword matched, `None` otherwise.
pub fn try_parse_heuristic(line: &str) -> Option<TaskEvent> {
    // Try error patterns first (higher priority)
    for pat in ERROR_PATTERNS.iter() {
        if let Some(caps) = pat.regex.captures(line) {
            return Some(build_event(line, &caps, "error", pat));
        }
    }
    // Then warning patterns
    for pat in WARNING_PATTERNS.iter() {
        if let Some(caps) = pat.regex.captures(line) {
            return Some(build_event(line, &caps, "warning", pat));
        }
    }
    None
}

struct HeuristicPattern {
    regex: Regex,
    file_group: Option<usize>,
    line_group: Option<usize>,
    col_group: Option<usize>,
}

fn build_event(
    line: &str,
    caps: &regex::Captures,
    severity: &str,
    pat: &HeuristicPattern,
) -> TaskEvent {
    let file = pat.file_group.and_then(|i| caps.get(i)).map(|m| m.as_str().to_string());
    let line_no =
        pat.line_group.and_then(|i| caps.get(i)).and_then(|m| m.as_str().parse::<u64>().ok());
    let col = pat.col_group.and_then(|i| caps.get(i)).and_then(|m| m.as_str().parse::<u64>().ok());

    let location = file.map(|f| EventLocation { file: f, line: line_no.unwrap_or(0), column: col });

    TaskEvent {
        seq: 0,
        event_type: "diagnostic".into(),
        severity: Some(severity.into()),
        code: None,
        message: line.to_string(),
        location,
        context: None,
    }
}

/// Error patterns: file:line:col prefix + error keyword, OR standalone error keywords.
static ERROR_PATTERNS: LazyLock<Vec<HeuristicPattern>> = LazyLock::new(|| {
    vec![
        // file:line:col: error: ...  (GCC/rustc/clippy style)
        HeuristicPattern {
            regex: Regex::new(r"^([^:\s]+):(\d+):(\d+):\s*(?i:(?:fatal\s+)?error)")
                .unwrap(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: Some(3),
        },
        // file:line: error: ...  (no column)
        HeuristicPattern {
            regex: Regex::new(r"^([^:\s]+):(\d+):\s*(?i:(?:fatal\s+)?error)").unwrap(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: None,
        },
        // Standalone error keywords (no file:line)
        // Uses leading \b only — trailing \b removed so "panicked" matches "panic"
        HeuristicPattern {
            regex: Regex::new(
                r"(?i)\b(?:fatal\s+error|FAILED|panic|traceback|segmentation fault|bus error|killed)",
            )
            .unwrap(),
            file_group: None,
            line_group: None,
            col_group: None,
        },
        // Generic "Error:" or "error:" at start or after whitespace
        HeuristicPattern {
            regex: Regex::new(r"(?i)(?:^|\s)error[\s:]+").unwrap(),
            file_group: None,
            line_group: None,
            col_group: None,
        },
    ]
});

/// Warning patterns: file:line:col prefix + warning keyword, OR standalone warning keywords.
static WARNING_PATTERNS: LazyLock<Vec<HeuristicPattern>> = LazyLock::new(|| {
    vec![
        // file:line:col: warning: ...
        HeuristicPattern {
            regex: Regex::new(r"^([^:\s]+):(\d+):(\d+):\s*(?i:warning)").unwrap(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: Some(3),
        },
        // file:line: warning: ...
        HeuristicPattern {
            regex: Regex::new(r"^([^:\s]+):(\d+):\s*(?i:warning)").unwrap(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: None,
        },
        // Standalone warning keywords
        HeuristicPattern {
            regex: Regex::new(r"(?i)\b(?:warning|deprecated)\b").unwrap(),
            file_group: None,
            line_group: None,
            col_group: None,
        },
    ]
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_with_file_line_col() {
        let evt = try_parse_heuristic("src/main.rs:42:10: error[E0308]: mismatched types")
            .expect("should match");
        assert_eq!(evt.event_type, "diagnostic");
        assert_eq!(evt.severity.as_deref(), Some("error"));
        let loc = evt.location.as_ref().expect("should have location");
        assert_eq!(loc.file, "src/main.rs");
        assert_eq!(loc.line, 42);
        assert_eq!(loc.column, Some(10));
    }

    #[test]
    fn error_with_file_line_no_col() {
        let evt =
            try_parse_heuristic("app.py:15: error: undefined name 'foo'").expect("should match");
        assert_eq!(evt.severity.as_deref(), Some("error"));
        let loc = evt.location.as_ref().expect("should have location");
        assert_eq!(loc.file, "app.py");
        assert_eq!(loc.line, 15);
        assert_eq!(loc.column, None);
    }

    #[test]
    fn fatal_error_keyword() {
        let evt = try_parse_heuristic("FATAL ERROR: out of memory").expect("should match");
        assert_eq!(evt.severity.as_deref(), Some("error"));
        assert!(evt.location.is_none());
    }

    #[test]
    fn panic_keyword() {
        let evt = try_parse_heuristic("thread 'main' panicked at 'index out of bounds'")
            .expect("should match");
        assert_eq!(evt.severity.as_deref(), Some("error"));
    }

    #[test]
    fn failed_keyword() {
        let evt = try_parse_heuristic("FAILED: 2/15 tests").expect("should match");
        assert_eq!(evt.severity.as_deref(), Some("error"));
    }

    #[test]
    fn warning_with_file_line() {
        let evt =
            try_parse_heuristic("lib.rs:10: warning: unused variable `x`").expect("should match");
        assert_eq!(evt.severity.as_deref(), Some("warning"));
        let loc = evt.location.as_ref().expect("should have location");
        assert_eq!(loc.file, "lib.rs");
        assert_eq!(loc.line, 10);
    }

    #[test]
    fn standalone_warning() {
        let evt = try_parse_heuristic("WARNING: this feature is deprecated").expect("should match");
        assert_eq!(evt.severity.as_deref(), Some("warning"));
    }

    #[test]
    fn no_match_on_normal_line() {
        assert!(try_parse_heuristic("hello world").is_none());
        assert!(try_parse_heuristic("Compiling foo v0.1.0").is_none());
        assert!(try_parse_heuristic("   Finished release [optimized]").is_none());
    }
}
