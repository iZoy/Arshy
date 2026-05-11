use arshy_lib::config::ParserConfig;
use arshy_lib::Result;

use super::{ParsedTool, ParserType};

/// A single parser entry in the registry.
#[derive(Debug, Clone)]
pub struct ParserEntry {
    pub name: String,
    pub tool_name: String,
    pub detect_patterns: Vec<String>,
    pub parser_type: ParserType,
    pub source: ParserSource,
    pub priority: u32,
}

#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq)]
pub enum ParserSource {
    Builtin,
    User,
}

/// Registry holding all available parsers, sorted by priority.
pub struct ParserRegistry {
    entries: Vec<ParserEntry>,
}

impl ParserRegistry {
    /// Load all parsers: builtin + from configured filesystem dirs.
    pub fn load(config: &ParserConfig) -> Result<Self> {
        let mut entries = builtin_parsers();

        for dir in &config.dirs {
            let expanded = arshy_lib::config::expand_path(dir);
            if expanded.is_dir() {
                if let Ok(files) = std::fs::read_dir(&expanded) {
                    for entry in files.flatten() {
                        if let Some(e) = load_from_file(&entry.path()) {
                            entries.push(e);
                        }
                    }
                }
            }
        }

        // Sort: higher priority first, user beats builtin on tie
        entries.sort_by(|a, b| {
            b.priority.cmp(&a.priority)
                .then_with(|| a.source.cmp(&b.source))
        });

        // First match wins — dedup by name
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(e.name.clone()));

        Ok(Self { entries })
    }

    /// Find the best parser matching a command.
    pub fn detect(&self, command: &str) -> Option<ParsedTool> {
        let cmd_name = command.split_whitespace().next()?.to_lowercase();

        for entry in &self.entries {
            if entry.parser_type == ParserType::Raw {
                continue; // Raw is fallback, never auto-detected
            }
            for pat in &entry.detect_patterns {
                if cmd_name.contains(&pat.to_lowercase()) {
                    return Some(ParsedTool {
                        tool_name: entry.tool_name.clone(),
                        parser_name: entry.name.clone(),
                        parser_type: entry.parser_type.clone(),
                        version: None,
                    });
                }
            }
        }
        None
    }
}

fn builtin_parsers() -> Vec<ParserEntry> {
    vec![
        ParserEntry {
            name: "tsc".into(), tool_name: "tsc".into(),
            detect_patterns: vec!["tsc".into()],
            parser_type: ParserType::Toml, source: ParserSource::Builtin, priority: 50,
        },
        ParserEntry {
            name: "vite".into(), tool_name: "vite".into(),
            detect_patterns: vec!["vite".into()],
            parser_type: ParserType::Toml, source: ParserSource::Builtin, priority: 50,
        },
        ParserEntry {
            name: "jest".into(), tool_name: "jest".into(),
            detect_patterns: vec!["jest".into()],
            parser_type: ParserType::Toml, source: ParserSource::Builtin, priority: 50,
        },
        ParserEntry {
            name: "cargo".into(), tool_name: "cargo".into(),
            detect_patterns: vec!["cargo".into()],
            parser_type: ParserType::Toml, source: ParserSource::Builtin, priority: 50,
        },
        ParserEntry {
            name: "npm".into(), tool_name: "npm".into(),
            detect_patterns: vec!["npm".into()],
            parser_type: ParserType::Rhai, source: ParserSource::Builtin, priority: 50,
        },
        ParserEntry {
            name: "raw".into(), tool_name: "*".into(),
            detect_patterns: vec![],
            parser_type: ParserType::Raw, source: ParserSource::Builtin, priority: 0,
        },
    ]
}

fn load_from_file(path: &std::path::Path) -> Option<ParserEntry> {
    let stem = path.file_stem()?.to_str()?;
    let ext = path.extension()?.to_str()?;
    let parser_type = match ext {
        "toml" => ParserType::Toml,
        "rhai" => ParserType::Rhai,
        _ => return None,
    };
    Some(ParserEntry {
        name: stem.to_string(),
        tool_name: stem.to_string(),
        detect_patterns: vec![stem.to_string()],
        parser_type,
        source: ParserSource::User,
        priority: 100,
    })
}
