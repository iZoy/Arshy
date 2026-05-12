//! Parser engine — TOML (Tier 1) + Stateful (Tier 2) + Raw fallback.
//!
//! Coverage target: 70% TOML regex, 25% stateful, 5% raw.

#[allow(unused_imports)]
mod detect;
mod loader;
mod registry;
pub mod rhai;
mod toml;

#[allow(unused_imports)]
pub use detect::*;
pub use registry::*;

use arshy_lib::config::ParserConfig;
use arshy_lib::ipc::TaskEvent;
use arshy_lib::Result;

use self::toml::TomlParser;

/// Central parser engine — detects tools, loads parsers, dispatches lines.
pub struct Engine {
    registry: ParserRegistry,
    #[allow(dead_code)]
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

    /// Create a parser session for a task. The session holds state for stateful parsers.
    pub fn create_session(&self, tool: Option<&ParsedTool>) -> ParserSession {
        let stateful = tool.and_then(|t| {
            if t.parser_type == ParserType::Rhai {
                Some(rhai::StatefulParser::new(&t.tool_name))
            } else {
                None
            }
        });
        ParserSession { stateful }
    }

    /// Parse a single output line into a structured event (stateless, for non-session use).
    pub fn parse_line(&self, line: &str, seq: u64, tool: Option<&ParsedTool>) -> TaskEvent {
        if let Some(t) = tool {
            match t.parser_type {
                ParserType::Toml => {
                    let parser = TomlParser::new(&t.parser_name);
                    if let Some(mut event) = parser.parse_line(line) {
                        event.seq = seq;
                        return event;
                    }
                }
                ParserType::Rhai => {
                    // Stateful parsing needs a session — fall back to raw here
                }
                ParserType::Raw => {}
            }
        }
        toml::raw_event(line, seq)
    }
}

/// A per-task parser session that holds state for stateful (Rhai-type) parsers.
///
/// Created via `Engine::create_session()`. For TOML parsers, delegates to
/// stateless parsing. For stateful parsers, maintains cross-line state.
pub struct ParserSession {
    stateful: Option<rhai::StatefulParser>,
}

impl ParserSession {
    /// Parse one line of output. Returns events (may be empty for stateful parsers
    /// that accumulate state across lines).
    pub fn parse_line(&self, line: &str, seq: u64, tool: Option<&ParsedTool>) -> Vec<TaskEvent> {
        if let Some(ref stateful) = self.stateful {
            return stateful.feed_line(line, seq);
        }

        // TOML / raw fallback
        if let Some(t) = tool {
            if t.parser_type == ParserType::Toml {
                let parser = TomlParser::new(&t.parser_name);
                if let Some(mut event) = parser.parse_line(line) {
                    event.seq = seq;
                    return vec![event];
                }
            }
        }

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
    pub parser_type: ParserType,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParserType {
    Toml,
    Rhai,
    Raw,
}
