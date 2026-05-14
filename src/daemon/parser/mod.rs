//! Parser engine — TOML (Tier 1) + Stateful (Tier 2) + Crash + Raw fallback.
//!
//! Coverage target: 70% TOML regex, 25% stateful, 5% crash/raw.
//!
//! Architecture:
//! - `toml_def` — TOML file format deserialization
//! - `toml` — stateless line-by-line matching
//! - `rhai` — stateful cross-line matching
//! - `crash` — universal crash/traceback detection
//! - `registry` — loads and deduplicates parser entries
//! - `detect` — tool extraction and version detection
//! - `loader` — filesystem hot-reload watcher

mod crash;
mod detect;
mod loader;
mod registry;
pub mod rhai;
pub mod toml;
pub mod toml_def;

pub use detect::*;
pub use registry::*;

use arshy_lib::config::ParserConfig;
use arshy_lib::ipc::TaskEvent;
use arshy_lib::Result;

use self::toml::LinePattern;

/// Central parser engine — detects tools, loads parsers, dispatches lines.
///
/// The engine is created once at daemon startup and shared (via `Arc`) across
/// all task executions. It holds the compiled `ParserRegistry` which contains
/// all builtin and user-defined parser patterns.
pub struct Engine {
    registry: ParserRegistry,
    config: ParserConfig,
}

impl Engine {
    /// Build the engine from config: load builtin + filesystem parsers.
    pub fn new(config: &ParserConfig) -> Result<Self> {
        let registry = ParserRegistry::load(config)?;
        Ok(Self { registry, config: config.clone() })
    }

    /// Detect the tool from a command string.
    pub fn detect(&self, command: &str) -> Option<ParsedTool> {
        self.registry.detect(command)
    }

    /// Create a parser session for a task.
    ///
    /// The session holds patterns from the registry and per-task state
    /// (for stateful parsers). Each task gets its own session instance.
    pub fn create_session(&self, tool: Option<&ParsedTool>) -> ParserSession {
        match tool {
            Some(t) => {
                if let Some(entry) = self.registry.get(&t.parser_name) {
                    return ParserSession {
                        line_patterns: entry.line_patterns.clone(),
                        stateful: if !entry.stateful_patterns.is_empty() {
                            Some(rhai::StatefulParser::with_patterns(
                                &entry.name,
                                entry.stateful_patterns.clone(),
                            ))
                        } else {
                            None
                        },
                    };
                }
                // Tool detected but no parser entry found — raw fallback
                ParserSession::raw()
            }
            None => ParserSession::raw(),
        }
    }

    /// Parse a single output line (stateless, for non-session use).
    #[allow(dead_code)] // stateless interface; session-based used in production
    pub fn parse_line(&self, line: &str, seq: u64, tool: Option<&ParsedTool>) -> TaskEvent {
        if let Some(t) = tool {
            if let Some(entry) = self.registry.get(&t.parser_name) {
                if !entry.line_patterns.is_empty() {
                    let parser = toml::TomlParser::new(entry.line_patterns.clone());
                    if let Some(mut event) = parser.parse_line(line) {
                        event.seq = seq;
                        return event;
                    }
                }
            }
        }
        toml::raw_event(line, seq)
    }

    pub fn config(&self) -> &ParserConfig {
        &self.config
    }
}

/// A per-task parser session that holds patterns and state.
///
/// Created via `Engine::create_session()`. For TOML parsers, uses stored
/// line patterns directly. For stateful parsers, maintains per-task state
/// in a `StatefulParser` instance.
pub struct ParserSession {
    line_patterns: Vec<LinePattern>,
    stateful: Option<rhai::StatefulParser>,
}

impl ParserSession {
    /// Create a raw (no-pattern) session for unrecognized commands.
    fn raw() -> Self {
        Self {
            line_patterns: Vec::new(),
            stateful: None,
        }
    }

    /// Parse one line of output.
    ///
    /// Strategy:
    /// 1. Stateful parser (if present) — may return 0..N events
    /// 2. Line patterns — first match wins, returns 0..1 events
    /// 3. Crash parser — universal crash/traceback detection
    /// 4. Raw fallback — every line becomes a log event
    pub fn parse_line(&self, line: &str, seq: u64, _tool: Option<&ParsedTool>) -> Vec<TaskEvent> {
        // 1. Stateful parser
        if let Some(ref stateful) = self.stateful {
            return stateful.feed_line(line, seq);
        }

        // 2. Line patterns (TOML parser)
        if !self.line_patterns.is_empty() {
            let parser = toml::TomlParser::new(self.line_patterns.clone());
            if let Some(mut event) = parser.parse_line(line) {
                event.seq = seq;
                return vec![event];
            }
        }

        // 3. Crash parser (universal, no tool dependency)
        if let Some(mut event) = crash::try_parse_crash(line) {
            event.seq = seq;
            return vec![event];
        }

        // 4. Raw fallback
        vec![toml::raw_event(line, seq)]
    }

    /// Called on command completion — emit final events from stateful parsers.
    pub fn on_complete(&self, exit_code: i32, seq: u64) -> Vec<TaskEvent> {
        if let Some(ref stateful) = self.stateful {
            return stateful.on_complete(exit_code, seq);
        }
        vec![]
    }
}

/// Metadata about a matched parser for a given command.
#[derive(Debug, Clone)]
pub struct ParsedTool {
    pub tool_name: String,
    pub parser_name: String,
    #[allow(dead_code)] // metadata for debugging/logging
    pub parser_type: ParserType,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParserType {
    Toml,
    Rhai,
    Raw,
}

// ── Parser harness tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod harness_tests {
    use super::*;

    /// Run a fixture test: parse `.txt` with the parser, compare with `.json`.
    fn run_fixture(parser_name: &str, txt_path: &std::path::Path, json_path: &std::path::Path) -> (usize, usize) {
        let config = ParserConfig::default();
        let engine = Engine::new(&config).unwrap();
        let tool = engine.detect(parser_name);
        let session = engine.create_session(tool.as_ref());

        let txt = std::fs::read_to_string(txt_path).unwrap();
        let json = std::fs::read_to_string(json_path).unwrap();
        let expected: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();

        let lines: Vec<&str> = txt.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut all_events: Vec<TaskEvent> = Vec::new();
        for line in &lines {
            all_events.extend(session.parse_line(line, 0, tool.as_ref()));
        }

        let mut matched = 0;
        for (i, exp) in expected.iter().enumerate() {
            if i >= all_events.len() {
                break;
            }
            let event = &all_events[i];
            let type_ok = exp.get("type").map_or(true, |v| v.as_str() == Some(&event.event_type));
            let sev_ok = exp.get("severity").map_or(true, |v| v.as_str() == event.severity.as_deref());
            let code_ok = exp.get("code").map_or(true, |v| v.as_str() == event.code.as_deref());
            let file_ok = exp.get("file").map_or(true, |v| {
                event.location.as_ref().map_or(false, |loc| v.as_str() == Some(&loc.file))
            });
            let line_ok = exp.get("line").map_or(true, |v| {
                event.location.as_ref().map_or(false, |loc| v.as_u64() == Some(loc.line))
            });

            if type_ok && sev_ok && code_ok && file_ok && line_ok {
                matched += 1;
            }
        }
        (expected.len(), matched)
    }

    fn run_parser_fixtures(parser_name: &str) {
        let base = std::path::Path::new("parsers/builtin/tests").join(parser_name);
        if !base.is_dir() {
            return;
        }

        let txt_files: Vec<_> = std::fs::read_dir(&base)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "txt"))
            .collect();

        assert!(!txt_files.is_empty(), "no fixtures in {}", base.display());

        for entry in &txt_files {
            let txt_path = entry.path();
            let json_path = txt_path.with_extension("json");
            let (total, matched) = run_fixture(parser_name, &txt_path, &json_path);
            let score = matched as f64 / total as f64;
            assert!(
                score >= 0.95,
                "parser '{}' fixture '{}': {:.0}% ({} / {})",
                parser_name,
                txt_path.file_stem().unwrap().to_str().unwrap(),
                score * 100.0,
                matched,
                total,
            );
        }
    }

    #[test]
    fn fixture_tsc() { run_parser_fixtures("tsc"); }

    #[test]
    fn fixture_cargo() { run_parser_fixtures("cargo"); }

    #[test]
    fn fixture_jest() { run_parser_fixtures("jest"); }

    #[test]
    fn all_20_parsers_load() {
        let config = ParserConfig::default();
        let engine = Engine::new(&config).unwrap();

        let parsers = [
            "tsc", "cargo", "jest", "vite", "eslint", "go", "python", "cc",
            "npm", "webpack", "prettier", "swc", "esbuild", "clippy", "make",
            "gradle", "cargo-test", "mocha", "pip", "pnpm",
        ];

        for name in &parsers {
            let tool = engine.detect(name);
            assert!(tool.is_some(), "parser '{}' not detected by engine", name);
        }
    }
}
