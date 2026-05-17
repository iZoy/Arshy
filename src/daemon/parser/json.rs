//! Generic JSON output parser.
//!
//! Detects JSON output from tools like `gh`, `kubectl`, `terraform`, `awscli`, etc.
//! When a command's output is valid JSON, it maps the structure into structured
//! TaskEvents instead of letting them fall through to raw text.

use arshy_lib::ipc::TaskEvent;

/// Attempts to parse a command's output as JSON and convert it to structured events.
///
/// Returns `None` if the output is not valid JSON (doesn't start with `{` or `[`,
/// or fails to parse). Returns `Some(events)` with structured events otherwise.
pub fn try_parse(output: &str) -> Option<Vec<TaskEvent>> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first_char = trimmed.chars().next()?;
    if first_char != '{' && first_char != '[' {
        return None;
    }

    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    match &value {
        serde_json::Value::Array(arr) => {
            let events: Vec<TaskEvent> = arr
                .iter()
                .map(|item| TaskEvent {
                    seq: 0, // caller assigns
                    event_type: "data".into(),
                    severity: Some("info".into()),
                    code: None,
                    message: if item.is_string() {
                        item.as_str().unwrap_or("").to_string()
                    } else {
                        serde_json::to_string(item).unwrap_or_default()
                    },
                    location: None,
                    context: None,
                })
                .collect();
            Some(events)
        }
        serde_json::Value::Object(_) => {
            let keys: Vec<&str> = value
                .as_object()
                .map(|obj| obj.keys().map(|k| k.as_str()).collect())
                .unwrap_or_default();
            let summary = format!("JSON object with fields: {}", keys.join(", "));
            Some(vec![TaskEvent {
                seq: 0,
                event_type: "data".into(),
                severity: Some("info".into()),
                code: None,
                message: serde_json::to_string_pretty(&value).unwrap_or(summary),
                location: None,
                context: None,
            }])
        }
        _ => Some(vec![TaskEvent {
            seq: 0,
            event_type: "data".into(),
            severity: Some("info".into()),
            code: None,
            message: serde_json::to_string(&value).unwrap_or_else(|_| trimmed.to_string()),
            location: None,
            context: None,
        }]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_output_returns_none() {
        assert!(try_parse("").is_none());
        assert!(try_parse("   ").is_none());
    }

    #[test]
    fn non_json_output_returns_none() {
        assert!(try_parse("hello world").is_none());
        assert!(try_parse("error: something went wrong").is_none());
        assert!(try_parse("success\n12 tests passed").is_none());
    }

    #[test]
    fn json_object_single_event() {
        let output = r#"{"status":"ok","count":42}"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "data");
        assert_eq!(events[0].severity.as_deref(), Some("info"));
        assert!(events[0].message.contains("status"));
        assert!(events[0].message.contains("count"));
    }

    #[test]
    fn json_array_multiple_events() {
        let output = r#"["one","two","three"]"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].message, "one");
        assert_eq!(events[1].message, "two");
        assert_eq!(events[2].message, "three");
    }

    #[test]
    fn json_array_of_objects() {
        let output = r#"[{"name":"alice"},{"name":"bob"}]"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[0].message.contains("alice"));
        assert!(events[1].message.contains("bob"));
    }

    #[test]
    fn json_with_leading_whitespace() {
        let output = "\n  {\"key\":\"value\"}";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn json_primitive_number() {
        let output = "42";
        assert!(try_parse(output).is_none());
    }

    #[test]
    fn invalid_json_returns_none() {
        assert!(try_parse(r#"{"broken": "#).is_none());
        assert!(try_parse(r#"["unclosed array"#).is_none());
    }

    #[test]
    fn nested_json_object() {
        let output = r#"{"items":[1,2,3],"meta":{"page":1}}"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].message.contains("items"));
        assert!(events[0].message.contains("meta"));
        assert!(events[0].message.contains("page"));
    }

    #[test]
    fn empty_json_array() {
        let output = "[]";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn empty_json_object() {
        let output = "{}";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 1);
    }
}
