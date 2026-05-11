//! Parser engine — TOML (Tier 1) + Rhai (Tier 2) + Raw fallback.
//!
//! Coverage target: 70% TOML, 25% Rhai, 5% Raw.

#[allow(unused_imports)]
mod detect;
mod loader;
mod registry;
mod rhai;
mod toml;

#[allow(unused_imports)]
pub use detect::*;
pub use registry::*;

use arshy_lib::config::ParserConfig;
use arshy_lib::Result;

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
