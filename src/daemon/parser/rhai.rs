//! Rhai script-based parser for stateful, cross-line parsing.
//!
//! Two modes:
//! - **Patterns**: regex+state-machine loaded from TOML definitions
//! - **Script**: `.rhai` files executed via the Rhai scripting engine
//!
//! # Rhai Script API
//!
//! ```rhai
//! fn on_line(line, ctx) {
//!     if line.contains("error") {
//!         ctx.emit("diagnostic", "error", line);
//!     }
//!     ctx.log("info", line);
//! }
//!
//! fn on_complete(exit_code, ctx) {
//!     if exit_code != 0 {
//!         ctx.emit("summary", "error", "command failed");
//!     }
//! }
//! ```

use arshy_lib::ipc::{EventLocation, TaskEvent};
use regex::Regex;

/// A stateful parser for cross-line pattern matching.
///
/// Two variants:
/// - `Patterns`: regex+state-machine from TOML definitions
/// - `Script`: user-defined `.rhai` script, compiled and executed per-call
///
/// The `Script` variant stores only the source text (not `rhai::Engine`/`AST`)
/// because rhai types use `Rc` internally and are `!Send`. A fresh engine is
/// created per call to keep `ParserSession` `Send`-compatible with tokio.
pub enum StatefulParser {
    #[allow(private_interfaces)]
    Patterns {
        name: String,
        patterns: Vec<StatefulPattern>,
        state: std::sync::Mutex<ParserState>,
    },
    #[allow(private_interfaces)]
    Script {
        source: String,
        state: std::sync::Mutex<ScriptState>,
    },
}

/// A regex pattern with state-machine transitions.
///
/// Used for tools whose output spans multiple lines and requires
/// tracking state across lines (e.g., npm error blocks, webpack chunks).
#[derive(Debug, Clone)]
pub struct StatefulPattern {
    pub regex: Regex,
    pub event_type: String,
    pub severity: String,
    pub message_group: Option<usize>,
    pub file_group: Option<usize>,
    pub line_group: Option<usize>,
    /// If set, this pattern only matches when state key has this value.
    pub state_condition: Option<(String, String)>,
    /// If set, transitions to this state after matching.
    pub state_transition: Option<(String, String)>,
}

#[derive(Default)]
struct ParserState {
    values: std::collections::HashMap<String, String>,
}

/// Send-compatible value type for script state (replaces `rhai::Dynamic`).
///
/// Supports the primitive types scripts typically store: strings, integers,
/// booleans. Floats are stored as integer representations to stay `Send`.
#[derive(Debug, Clone)]
enum SendValue {
    Str(String),
    Int(i64),
    Bool(bool),
    Unit,
}

impl SendValue {
    fn to_dynamic(&self) -> rhai::Dynamic {
        match self {
            SendValue::Str(s) => rhai::Dynamic::from(s.clone()),
            SendValue::Int(i) => rhai::Dynamic::from(*i),
            SendValue::Bool(b) => rhai::Dynamic::from(*b),
            SendValue::Unit => rhai::Dynamic::UNIT,
        }
    }

    fn from_dynamic(d: &rhai::Dynamic) -> Self {
        if let Ok(b) = d.as_bool() {
            return SendValue::Bool(b);
        }
        if let Ok(i) = d.as_int() {
            return SendValue::Int(i);
        }
        // Fall back to string representation.
        let s = d.to_string();
        if s == "()" {
            SendValue::Unit
        } else {
            SendValue::Str(s)
        }
    }
}

/// State for rhai script parsers — persisted key-value state across lines.
struct ScriptState {
    values: std::collections::HashMap<String, SendValue>,
}

/// Context object passed to rhai scripts as `ctx`.
///
/// Provides `emit()`, `set()`, `get()`, `log()` functions.
/// Uses `Rc<RefCell<...>>` for sharing between ctx and the calling code;
/// the `Send` constraint is satisfied by storing only `SendValue` (not `Dynamic`).
#[derive(Clone)]
struct RhaiCtx {
    events: std::rc::Rc<std::cell::RefCell<Vec<TaskEvent>>>,
    values: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, SendValue>>>,
    seq: u64,
}

impl StatefulParser {
    /// Create with pre-loaded patterns (from TOML definitions via registry).
    pub fn with_patterns(name: &str, patterns: Vec<StatefulPattern>) -> Self {
        Self::Patterns {
            name: name.to_string(),
            patterns,
            state: std::sync::Mutex::new(ParserState::default()),
        }
    }

    /// Create with a rhai script (from `.rhai` file).
    /// Validates syntax at creation time; runtime engine is created per-call.
    pub fn with_script(script: &str) -> Result<Self, String> {
        // Validate syntax with a temporary engine.
        let engine = rhai::Engine::new();
        engine
            .compile(script)
            .map_err(|e| format!("rhai compile error: {}", e))?;
        Ok(Self::Script {
            source: script.to_string(),
            state: std::sync::Mutex::new(ScriptState {
                values: std::collections::HashMap::new(),
            }),
        })
    }

    /// Legacy constructor using builtin patterns. Prefer `with_patterns`.
    #[allow(dead_code)]
    pub fn new(name: &str) -> Self {
        Self::with_patterns(name, builtin_stateful_patterns(name))
    }

    /// Parser name (only available for pattern-based parsers).
    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        match self {
            Self::Patterns { name, .. } => name,
            Self::Script { .. } => "rhai",
        }
    }

    /// Feed one line to the stateful parser. Returns any events generated.
    pub fn feed_line(&self, line: &str, seq: u64) -> Vec<TaskEvent> {
        match self {
            Self::Patterns { patterns, state, .. } => {
                feed_patterns(patterns, state, line, seq)
            }
            Self::Script { source, state, .. } => {
                feed_script(source, state, line, seq)
            }
        }
    }

    /// Called on command completion — emit final events.
    pub fn on_complete(&self, exit_code: i32, seq: u64) -> Vec<TaskEvent> {
        match self {
            Self::Patterns { name, state, .. } => {
                on_complete_patterns(name, state, exit_code, seq)
            }
            Self::Script { source, state, .. } => {
                on_complete_script(source, state, exit_code, seq)
            }
        }
    }

    /// Reset parser state (for reuse).
    #[allow(dead_code)]
    pub fn reset(&self) {
        match self {
            Self::Patterns { state, .. } => {
                *state.lock().unwrap() = ParserState::default();
            }
            Self::Script { state, .. } => {
                let mut s = state.lock().unwrap();
                s.values.clear();
            }
        }
    }
}

// ── Pattern-based implementation ──────────────────────────────────────────────

fn feed_patterns(
    patterns: &[StatefulPattern],
    state: &std::sync::Mutex<ParserState>,
    line: &str,
    seq: u64,
) -> Vec<TaskEvent> {
    let mut events = Vec::new();
    let mut state = state.lock().unwrap();

    for pat in patterns {
        if let Some((key, value)) = &pat.state_condition {
            match state.values.get(key.as_str()) {
                Some(v) if v == value => {}
                _ => continue,
            }
        }

        if let Some(caps) = pat.regex.captures(line) {
            let message = pat.message_group
                .and_then(|i| caps.get(i))
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_else(|| line.to_string());

            let location = if let Some(fi) = pat.file_group {
                caps.get(fi).map(|m| EventLocation {
                    file: m.as_str().to_string(),
                    line: pat.line_group
                        .and_then(|li| caps.get(li))
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0),
                    column: None,
                })
            } else {
                None
            };

            events.push(TaskEvent {
                seq: seq + events.len() as u64,
                event_type: pat.event_type.clone(),
                severity: Some(pat.severity.clone()),
                code: None,
                message,
                location,
                context: None,
            });

            if let Some((key, value)) = &pat.state_transition {
                state.values.insert(key.clone(), value.clone());
            }
        }
    }

    events
}

fn on_complete_patterns(
    name: &str,
    state: &std::sync::Mutex<ParserState>,
    exit_code: i32,
    seq: u64,
) -> Vec<TaskEvent> {
    let mut events = Vec::new();

    if name == "npm" {
        let state = state.lock().unwrap();
        let has_adds = state.values.contains_key("packages_added");
        let has_error = state.values.get("has_error").is_some_and(|v| v == "true");

        if has_adds || has_error {
            let severity = if has_error || exit_code != 0 { "error" } else { "info" };
            let msg = if has_adds {
                format!("npm install completed (exit {})", exit_code)
            } else {
                format!("npm install failed (exit {})", exit_code)
            };
            events.push(TaskEvent {
                seq,
                event_type: "summary".into(),
                severity: Some(severity.into()),
                code: None,
                message: msg,
                location: None,
                context: None,
            });
        }
    }

    events
}

// ── Script-based implementation ───────────────────────────────────────────────

fn register_ctx_api(engine: &mut rhai::Engine) {
    engine.register_type::<RhaiCtx>();

    engine.register_fn("emit", |ctx: &mut RhaiCtx, typ: &str, sev: &str, msg: &str| {
        let mut evts = ctx.events.borrow_mut();
        let seq = ctx.seq + evts.len() as u64;
        evts.push(TaskEvent {
            seq,
            event_type: typ.to_string(),
            severity: Some(sev.to_string()),
            code: None,
            message: msg.to_string(),
            location: None,
            context: None,
        });
    });

    engine.register_fn("emit", |ctx: &mut RhaiCtx, typ: &str, sev: &str, msg: &str, file: &str, line: i64| {
        let mut evts = ctx.events.borrow_mut();
        let seq = ctx.seq + evts.len() as u64;
        evts.push(TaskEvent {
            seq,
            event_type: typ.to_string(),
            severity: Some(sev.to_string()),
            code: None,
            message: msg.to_string(),
            location: Some(EventLocation {
                file: file.to_string(),
                line: line as u64,
                column: None,
            }),
            context: None,
        });
    });

    engine.register_fn("set", |ctx: &mut RhaiCtx, key: &str, value: rhai::Dynamic| {
        ctx.values.borrow_mut().insert(key.to_string(), SendValue::from_dynamic(&value));
    });

    engine.register_fn("get", |ctx: &mut RhaiCtx, key: &str| -> rhai::Dynamic {
        ctx.values.borrow().get(key).cloned().map(|v| v.to_dynamic()).unwrap_or(rhai::Dynamic::UNIT)
    });

    engine.register_fn("has", |ctx: &mut RhaiCtx, key: &str| -> bool {
        ctx.values.borrow().contains_key(key)
    });
}

fn feed_script(
    source: &str,
    state: &std::sync::Mutex<ScriptState>,
    line: &str,
    seq: u64,
) -> Vec<TaskEvent> {
    let events_rc: std::rc::Rc<std::cell::RefCell<Vec<TaskEvent>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let values_rc: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, SendValue>>> =
        std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new()));

    {
        let st = state.lock().unwrap();
        for (k, v) in &st.values {
            values_rc.borrow_mut().insert(k.clone(), v.clone());
        }
    }

    let ctx = RhaiCtx {
        events: events_rc.clone(),
        values: values_rc.clone(),
        seq,
    };

    let mut engine = rhai::Engine::new();
    register_ctx_api(&mut engine);

    let ast = match engine.compile(source) {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!("rhai compile error: {}", e);
            return Vec::new();
        }
    };

    let mut scope = rhai::Scope::new();
    let line_owned = line.to_string();
    let result: Result<(), _> = engine.call_fn(&mut scope, &ast, "on_line", (line_owned, ctx));
    if let Err(e) = result {
        tracing::debug!("rhai on_line error: {}", e);
    }

    {
        let mut st = state.lock().unwrap();
        st.values = values_rc.borrow().clone();
    }

    std::rc::Rc::try_unwrap(events_rc)
        .map(|c| c.into_inner())
        .unwrap_or_else(|rc| rc.borrow().clone())
}

fn on_complete_script(
    source: &str,
    state: &std::sync::Mutex<ScriptState>,
    exit_code: i32,
    seq: u64,
) -> Vec<TaskEvent> {
    let events_rc: std::rc::Rc<std::cell::RefCell<Vec<TaskEvent>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let values_rc: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, SendValue>>> =
        std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashMap::new()));

    {
        let st = state.lock().unwrap();
        for (k, v) in &st.values {
            values_rc.borrow_mut().insert(k.clone(), v.clone());
        }
    }

    let ctx = RhaiCtx {
        events: events_rc.clone(),
        values: values_rc.clone(),
        seq,
    };

    let mut engine = rhai::Engine::new();
    register_ctx_api(&mut engine);

    let ast = match engine.compile(source) {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!("rhai compile error: {}", e);
            return Vec::new();
        }
    };

    let mut scope = rhai::Scope::new();
    let result: Result<(), _> = engine.call_fn(&mut scope, &ast, "on_complete", (exit_code as i64, ctx));
    if let Err(e) = result {
        tracing::debug!("rhai on_complete error: {}", e);
    }

    {
        let mut st = state.lock().unwrap();
        st.values = values_rc.borrow().clone();
    }

    std::rc::Rc::try_unwrap(events_rc)
        .map(|c| c.into_inner())
        .unwrap_or_else(|rc| rc.borrow().clone())
}

/// Built-in stateful patterns for cross-line tools.
fn builtin_stateful_patterns(tool: &str) -> Vec<StatefulPattern> {
    match tool {
        "npm" => npm_stateful_patterns(),
        "webpack" => webpack_patterns(),
        _ => vec![],
    }
}

fn npm_stateful_patterns() -> Vec<StatefulPattern> {
    vec![
        // added 42 packages in 3s
        StatefulPattern {
            regex: Regex::new(r"^added (\d+) packages?").unwrap(),
            event_type: "summary".into(),
            severity: "info".into(),
            message_group: None,
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: Some(("packages_added".into(), "done".into())),
        },
        // npm ERR! code ERESOLVE
        StatefulPattern {
            regex: Regex::new(r"^npm ERR! (.+)").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            message_group: Some(1),
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: Some(("has_error".into(), "true".into())),
        },
        // npm WARN deprecated ...
        StatefulPattern {
            regex: Regex::new(r"^npm WARN (.+)").unwrap(),
            event_type: "diagnostic".into(),
            severity: "warning".into(),
            message_group: Some(1),
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: None,
        },
        // up to date, audited X packages
        StatefulPattern {
            regex: Regex::new(r"^up to date,? audited (\d+) packages?").unwrap(),
            event_type: "summary".into(),
            severity: "info".into(),
            message_group: None,
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: None,
        },
        // audited X packages in Ys
        StatefulPattern {
            regex: Regex::new(r"^audited (\d+) packages?").unwrap(),
            event_type: "summary".into(),
            severity: "info".into(),
            message_group: None,
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: None,
        },
    ]
}

fn webpack_patterns() -> Vec<StatefulPattern> {
    vec![
        // ERROR in ./src/index.ts
        StatefulPattern {
            regex: Regex::new(r"^ERROR in (.+)").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            message_group: Some(1),
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: Some(("has_error".into(), "true".into())),
        },
        // WARNING in ./src/index.ts
        StatefulPattern {
            regex: Regex::new(r"^WARNING in (.+)").unwrap(),
            event_type: "diagnostic".into(),
            severity: "warning".into(),
            message_group: Some(1),
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Pattern-based tests ───────────────────────────────────────────────

    #[test]
    fn test_npm_error() {
        let parser = StatefulParser::new("npm");
        let events = parser.feed_line("npm ERR! code ERESOLVE", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].severity, Some("error".into()));
        assert!(events[0].message.contains("ERESOLVE"));
    }

    #[test]
    fn test_npm_warn() {
        let parser = StatefulParser::new("npm");
        let events = parser.feed_line("npm WARN deprecated request@2.88.2", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].severity, Some("warning".into()));
    }

    #[test]
    fn test_npm_summary() {
        let parser = StatefulParser::new("npm");
        let events = parser.feed_line("added 42 packages in 3s", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "summary");
    }

    #[test]
    fn test_npm_on_complete_with_error() {
        let parser = StatefulParser::new("npm");
        parser.feed_line("npm ERR! code ENOENT", 1);
        let events = parser.on_complete(1, 2);
        assert_eq!(events.len(), 1);
        assert!(events[0].message.contains("failed"));
        assert_eq!(events[0].severity, Some("error".into()));
    }

    #[test]
    fn test_npm_on_complete_success() {
        let parser = StatefulParser::new("npm");
        parser.feed_line("added 42 packages in 3s", 1);
        let events = parser.on_complete(0, 2);
        assert_eq!(events.len(), 1);
        assert!(events[0].message.contains("completed"));
        assert_eq!(events[0].severity, Some("info".into()));
    }

    #[test]
    fn test_webpack_error() {
        let parser = StatefulParser::new("webpack");
        let events = parser.feed_line("ERROR in ./src/index.ts", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].severity, Some("error".into()));
    }

    #[test]
    fn test_unknown_tool_no_patterns() {
        let parser = StatefulParser::new("unknown_tool");
        let events = parser.feed_line("some output line", 1);
        assert!(events.is_empty());
    }

    // ── Script-based tests ────────────────────────────────────────────────

    #[test]
    fn test_script_basic_emit() {
        let script = r#"
fn on_line(line, ctx) {
    if line.contains("error") {
        ctx.emit("diagnostic", "error", line);
    }
}

fn on_complete(exit_code, ctx) {}
"#;
        let parser = StatefulParser::with_script(script).unwrap();
        let events = parser.feed_line("something went error here", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "diagnostic");
        assert_eq!(events[0].severity, Some("error".into()));
    }

    #[test]
    fn test_script_state_persistence() {
        let script = r#"
fn on_line(line, ctx) {
    if line.contains("count:") {
        ctx.set("seen", true);
    }
}

fn on_complete(exit_code, ctx) {
    if ctx.has("seen") {
        ctx.emit("summary", "info", "saw count");
    }
}
"#;
        let parser = StatefulParser::with_script(script).unwrap();
        parser.feed_line("count: 42", 1);
        let events = parser.on_complete(0, 2);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message, "saw count");
    }

    #[test]
    fn test_script_emit_with_location() {
        let script = r#"
fn on_line(line, ctx) {
    ctx.emit("diagnostic", "error", "err msg", "main.rs", 42);
}

fn on_complete(exit_code, ctx) {}
"#;
        let parser = StatefulParser::with_script(script).unwrap();
        let events = parser.feed_line("anything", 1);
        assert_eq!(events.len(), 1);
        let loc = events[0].location.as_ref().unwrap();
        assert_eq!(loc.file, "main.rs");
        assert_eq!(loc.line, 42);
    }

    #[test]
    fn test_script_no_match() {
        let script = r#"
fn on_line(line, ctx) {}

fn on_complete(exit_code, ctx) {}
"#;
        let parser = StatefulParser::with_script(script).unwrap();
        let events = parser.feed_line("nothing matches", 1);
        assert!(events.is_empty());
    }

    #[test]
    fn test_script_compile_error() {
        let script = "fn broken( { invalid";
        let result = StatefulParser::with_script(script);
        assert!(result.is_err());
    }
}
