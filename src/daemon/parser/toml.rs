use arshy_lib::ipc::{EventLocation, TaskEvent};
use arshy_lib::Result;
use regex::Regex;

/// A TOML-based line-matching parser.
///
/// For v1.0, parsers are compiled with built-in regex patterns.
/// Future: load patterns from TOML definition files.
pub struct TomlParser {
    pub name: String,
    patterns: Vec<LinePattern>,
}

/// A single regex pattern that matches a line and extracts event fields.
struct LinePattern {
    regex: Regex,
    event_type: String,
    severity: String,
    file_group: Option<usize>,
    line_group: Option<usize>,
    col_group: Option<usize>,
    code_group: Option<usize>,
    message_group: Option<usize>,
}

impl TomlParser {
    /// Create a parser with built-in patterns for the given tool.
    pub fn new(name: &str) -> Self {
        let patterns = builtin_patterns(name);
        Self { name: name.to_string(), patterns }
    }

    /// Parse one line of output. Returns Some(event) if any pattern matched.
    pub fn parse_line(&self, line: &str) -> Option<TaskEvent> {
        for pat in &self.patterns {
            if let Some(caps) = pat.regex.captures(line) {
                let file = pat.file_group.and_then(|i| caps.get(i)).map(|m| m.as_str().to_string());
                let line_no = pat.line_group
                    .and_then(|i| caps.get(i))
                    .and_then(|m| m.as_str().parse::<u64>().ok());
                let col = pat.col_group
                    .and_then(|i| caps.get(i))
                    .and_then(|m| m.as_str().parse::<u64>().ok());
                let code = pat.code_group
                    .and_then(|i| caps.get(i))
                    .map(|m| m.as_str().to_string());
                let message = pat.message_group
                    .and_then(|i| caps.get(i))
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_else(|| line.to_string());

                let location = if let Some(f) = file {
                    Some(EventLocation { file: f, line: line_no.unwrap_or(0), column: col })
                } else {
                    None
                };

                return Some(TaskEvent {
                    seq: 0, // caller sets seq
                    event_type: pat.event_type.clone(),
                    severity: Some(pat.severity.clone()),
                    code,
                    message,
                    location,
                    context: None,
                });
            }
        }
        None
    }
}

/// Raw fallback — every line becomes a log event.
pub fn raw_event(line: &str, seq: u64) -> TaskEvent {
    // Detect common error/warning prefixes for severity inference
    let severity = if line.contains("error") || line.contains("Error") || line.contains("ERROR") || line.contains("fatal") {
        "error"
    } else if line.contains("warning") || line.contains("Warning") || line.contains("WARN") {
        "warning"
    } else {
        "info"
    };

    TaskEvent {
        seq,
        event_type: "log".into(),
        severity: Some(severity.into()),
        code: None,
        message: line.to_string(),
        location: None,
        context: None,
    }
}

/// Built-in regex patterns for common CLI tools.
fn builtin_patterns(tool: &str) -> Vec<LinePattern> {
    match tool {
        "tsc" => tsc_patterns(),
        "cargo" => cargo_patterns(),
        "jest" => jest_patterns(),
        "vite" => vite_patterns(),
        _ => vec![],
    }
}

/// TypeScript compiler error patterns.
fn tsc_patterns() -> Vec<LinePattern> {
    vec![
        // src/app.ts(10,5): error TS2322: Type 'string' is not assignable to type 'number'.
        LinePattern {
            regex: Regex::new(r"^(.+?)\((\d+),(\d+)\): error (TS\d+): (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: Some(3),
            code_group: Some(4),
            message_group: Some(5),
        },
        // src/app.ts(10,5): warning TS2322: ...
        LinePattern {
            regex: Regex::new(r"^(.+?)\((\d+),(\d+)\): warning (TS\d+): (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "warning".into(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: Some(3),
            code_group: Some(4),
            message_group: Some(5),
        },
        // src/app.ts:10:5 - error TS2322: ... (alternative format)
        LinePattern {
            regex: Regex::new(r"^(.+?):(\d+):(\d+) - error (TS\d+): (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: Some(3),
            code_group: Some(4),
            message_group: Some(5),
        },
        // Found X errors.
        LinePattern {
            regex: Regex::new(r"^Found (\d+) errors?\.$").unwrap(),
            event_type: "summary".into(),
            severity: "error".into(),
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: None,
        },
    ]
}

/// Cargo build error patterns.
fn cargo_patterns() -> Vec<LinePattern> {
    vec![
        // error[E0308]: mismatched types
        LinePattern {
            regex: Regex::new(r"^error\[([E]\d+)\]: (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: Some(1),
            message_group: Some(2),
        },
        // --> src/main.rs:10:5
        LinePattern {
            regex: Regex::new(r"^\s*--> (.+?):(\d+):(\d+)$").unwrap(),
            event_type: "location".into(),
            severity: "info".into(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: Some(3),
            code_group: None,
            message_group: None,
        },
        // warning: unused variable
        LinePattern {
            regex: Regex::new(r"^warning: (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "warning".into(),
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: Some(1),
        },
    ]
}

/// Jest test output patterns.
fn jest_patterns() -> Vec<LinePattern> {
    vec![
        // FAIL src/components/Button.test.tsx
        LinePattern {
            regex: Regex::new(r"^FAIL (.+)$").unwrap(),
            event_type: "test_result".into(),
            severity: "error".into(),
            file_group: Some(1),
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: None,
        },
        // PASS src/components/Button.test.tsx
        LinePattern {
            regex: Regex::new(r"^PASS (.+)$").unwrap(),
            event_type: "test_result".into(),
            severity: "info".into(),
            file_group: Some(1),
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: None,
        },
        // Tests:       1 failed, 2 passed, 3 total
        LinePattern {
            regex: Regex::new(r"^Tests:\s+(?:(\d+) failed,\s*)?(?:(\d+) passed,\s*)?(\d+) total").unwrap(),
            event_type: "summary".into(),
            severity: "info".into(), // will be overridden if failures > 0
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: None,
        },
    ]
}

/// Vite build output patterns.
fn vite_patterns() -> Vec<LinePattern> {
    vec![
        // error: ...
        LinePattern {
            regex: Regex::new(r"^error: (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: Some(1),
        },
        // Build failed with errors.
        LinePattern {
            regex: Regex::new(r"^Build failed").unwrap(),
            event_type: "summary".into(),
            severity: "error".into(),
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tsc_error_pattern() {
        let parser = TomlParser::new("tsc");
        let event = parser.parse_line("src/app.ts(10,5): error TS2322: Type 'string' is not assignable to type 'number'.");
        assert!(event.is_some());
        let e = event.unwrap();
        assert_eq!(e.severity, Some("error".into()));
        assert_eq!(e.code, Some("TS2322".into()));
        let loc = e.location.unwrap();
        assert_eq!(loc.file, "src/app.ts");
        assert_eq!(loc.line, 10);
        assert_eq!(loc.column, Some(5));
    }

    #[test]
    fn test_tsc_warning_pattern() {
        let parser = TomlParser::new("tsc");
        let event = parser.parse_line("src/util.ts(3,1): warning TS6133: 'x' is declared but never used.");
        assert!(event.is_some());
        let e = event.unwrap();
        assert_eq!(e.severity, Some("warning".into()));
    }

    #[test]
    fn test_tsc_found_errors() {
        let parser = TomlParser::new("tsc");
        let event = parser.parse_line("Found 3 errors.");
        assert!(event.is_some());
        let e = event.unwrap();
        assert_eq!(e.event_type, "summary");
    }

    #[test]
    fn test_cargo_error() {
        let parser = TomlParser::new("cargo");
        let event = parser.parse_line("error[E0308]: mismatched types");
        assert!(event.is_some());
        let e = event.unwrap();
        assert_eq!(e.code, Some("E0308".into()));
        assert_eq!(e.severity, Some("error".into()));
    }

    #[test]
    fn test_jest_fail() {
        let parser = TomlParser::new("jest");
        let event = parser.parse_line("FAIL src/components/Button.test.tsx");
        assert!(event.is_some());
        let e = event.unwrap();
        assert_eq!(e.event_type, "test_result");
        assert_eq!(e.severity, Some("error".into()));
    }

    #[test]
    fn test_raw_fallback() {
        let event = raw_event("hello world", 1);
        assert_eq!(event.event_type, "log");
        assert_eq!(event.severity, Some("info".into()));
        assert_eq!(event.message, "hello world");
    }

    #[test]
    fn test_raw_severity_detection() {
        let err = raw_event("Error: something failed", 1);
        assert_eq!(err.severity, Some("error".into()));

        let warn = raw_event("Warning: deprecated usage", 2);
        assert_eq!(warn.severity, Some("warning".into()));
    }
}
