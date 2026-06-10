//! Universal crash/traceback parser — detects cross-language crash patterns.
//!
//! This is a lightweight, always-active parser that runs as a fallback
//! when the tool-specific parser doesn't match a line. It catches common
//! crash signatures across Go, Python, Node.js, Rust, and shell.
//!
//! Not a full parser — just enough to emit a structured `crash` event
//! with language, error type, file, and line number.

use arshy_lib::ipc::{EventLocation, TaskEvent};
use regex::Regex;
use std::sync::LazyLock;

/// Try to match a line as a crash/traceback pattern.
/// Returns `Some(event)` if matched, `None` otherwise.
pub fn try_parse_crash(line: &str) -> Option<TaskEvent> {
    for pattern in PATTERNS.iter() {
        if let Some(caps) = pattern.regex.captures(line) {
            let message = pattern
                .message_group
                .and_then(|i| caps.get(i))
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_else(|| line.to_string());

            let file = pattern.file_group.and_then(|i| caps.get(i)).map(|m| m.as_str().to_string());

            let line_no = pattern
                .line_group
                .and_then(|i| caps.get(i))
                .and_then(|m| m.as_str().parse::<u64>().ok());

            let location =
                file.map(|f| EventLocation { file: f, line: line_no.unwrap_or(0), column: None });

            return Some(TaskEvent {
                seq: 0,
                event_type: "crash".into(),
                severity: Some("error".into()),
                code: Some(pattern.language.into()),
                message,
                location,
                context: None,
                hint: None,
            });
        }
    }
    None
}

// ── Internal pattern definition ──────────────────────────────────────────────

struct CrashPattern {
    language: &'static str,
    regex: Regex,
    file_group: Option<usize>,
    line_group: Option<usize>,
    message_group: Option<usize>,
}

/// Static crash patterns, compiled once via `Lazy`.
static PATTERNS: LazyLock<Vec<CrashPattern>> = LazyLock::new(|| {
    vec![
        // Go panic: "panic: runtime error: ..." followed by "goroutine N [running]:"
        // We match the source line: "main.go:42 +0x1234" (space) or "main.go:42:10" (colon)
        CrashPattern {
            language: "go",
            regex: Regex::new(r"^(.+\.go):(\d+)(?::\d+)?\s+(.+)$").unwrap(),
            file_group: Some(1),
            line_group: Some(2),
            message_group: Some(3),
        },
        // Python traceback: '  File "app.py", line 42'
        CrashPattern {
            language: "python",
            regex: Regex::new(r#"^\s*File "(.+?)", line (\d+)"#).unwrap(),
            file_group: Some(1),
            line_group: Some(2),
            message_group: None,
        },
        // Python exception line: explicit list of Python built-in exception types.
        // Checked BEFORE Node.js errors so Python exceptions are correctly classified.
        CrashPattern {
            language: "python",
            regex: Regex::new(r"^(?:(?:Type|Value|Key|Index|Attribute|Runtime|OS|IO|Import|Name|Syntax|ZeroDivision|Memory|Recursion|StopIteration|Assertion|Permission|Timeout|Process|Unicode|UnicodeEncode|UnicodeDecode|UnicodeTranslate|SystemExit|KeyboardInterrupt|GeneratorExit|BlockingIO|BrokenPipe|ChildProcess|ConnectionAborted|ConnectionRefused|ConnectionReset|FileExists|FileNotFound|IsADirectory|NotADirectory|Interrupted|Overflow|Arithmetic|FloatingPoint|Buffer|Lookup|Environment|EOF|UnboundLocal|Indentation|Tab|Deprecation|PendingDeprecation|ResourceWarning)(?:Error|Exception|Warning)):\s+(.+)$").unwrap(),
            file_group: None,
            line_group: None,
            message_group: Some(1),
        },
        // Rust panic: "thread 'main' panicked at 'message', src/main.rs:42:5"
        CrashPattern {
            language: "rust",
            regex: Regex::new(r"^thread '.+' panicked at (.+), (.+):(\d+):").unwrap(),
            file_group: Some(2),
            line_group: Some(3),
            message_group: Some(1),
        },
        // Node.js error: "Error: ..." / "RangeError: ..." / "URIError: ..."
        // Python exceptions are already caught by the pattern above.
        CrashPattern {
            language: "node",
            regex: Regex::new(r"^(\w*Error): (.+)$").unwrap(),
            file_group: None,
            line_group: None,
            message_group: Some(2),
        },
        // Node.js stack frame: "    at func (file.js:42:10)"
        CrashPattern {
            language: "node",
            regex: Regex::new(r"^\s+at .+ \((.+):(\d+):\d+\)").unwrap(),
            file_group: Some(1),
            line_group: Some(2),
            message_group: None,
        },
        // Shell segfault / signal
        CrashPattern {
            language: "shell",
            regex: Regex::new(r"^(Segmentation fault|Bus error|Abort trap|Killed)$").unwrap(),
            file_group: None,
            line_group: None,
            message_group: None,
        },
    ]
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_go_source_line() {
        let event = try_parse_crash("main.go:42 +0x1234").unwrap();
        assert_eq!(event.code, Some("go".into()));
        assert_eq!(event.event_type, "crash");
        let loc = event.location.unwrap();
        assert_eq!(loc.file, "main.go");
        assert_eq!(loc.line, 42);
    }

    #[test]
    fn test_python_traceback() {
        let event = try_parse_crash(r#"  File "app.py", line 10"#).unwrap();
        assert_eq!(event.code, Some("python".into()));
        let loc = event.location.unwrap();
        assert_eq!(loc.file, "app.py");
        assert_eq!(loc.line, 10);
    }

    #[test]
    fn test_python_exception() {
        let event = try_parse_crash("TypeError: cannot read property 'x' of undefined").unwrap();
        assert_eq!(event.code, Some("python".into()));
        assert_eq!(event.message, "cannot read property 'x' of undefined");
    }

    #[test]
    fn test_rust_panic() {
        let event =
            try_parse_crash("thread 'main' panicked at 'index out of bounds', src/main.rs:42:5")
                .unwrap();
        assert_eq!(event.code, Some("rust".into()));
        let loc = event.location.unwrap();
        assert_eq!(loc.file, "src/main.rs");
        assert_eq!(loc.line, 42);
    }

    #[test]
    fn test_node_error() {
        let event = try_parse_crash("ReferenceError: myVar is not defined").unwrap();
        assert_eq!(event.code, Some("node".into()));
        assert!(event.message.contains("myVar is not defined"));
    }

    #[test]
    fn test_node_stack_frame() {
        let event = try_parse_crash("    at processCallback (index.js:42:10)").unwrap();
        assert_eq!(event.code, Some("node".into()));
        let loc = event.location.unwrap();
        assert_eq!(loc.file, "index.js");
        assert_eq!(loc.line, 42);
    }

    #[test]
    fn test_segmentation_fault() {
        let event = try_parse_crash("Segmentation fault").unwrap();
        assert_eq!(event.code, Some("shell".into()));
        assert_eq!(event.severity, Some("error".into()));
    }

    #[test]
    fn test_no_match() {
        assert!(try_parse_crash("hello world").is_none());
        assert!(try_parse_crash("all tests passed").is_none());
    }
}
