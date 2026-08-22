//! Reference tables for non-obvious error codes.
//!
//! This is the "restricted reference" resolution of the old HintDb
//! question: reference **data only** — what a code *means* (docker
//! 125/126/127/137, kubectl/aws exit codes) — never cause/fix advice.
//! Entries are served on demand through `task/query`, never inlined into
//! the stored event stream (`TaskEvent.hint` stays a null placeholder).
//! What to *do* about a code remains the LLM's job.
//!
//! Sources (in priority order):
//! 1. Builtin tables (embedded TOML, `reference/builtin/*.toml`)
//! 2. User tables (`~/.arshy/reference/*.toml`), same `meta.name` overrides

use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
struct ReferenceFile {
    #[serde(default)]
    meta: ReferenceMeta,
    #[serde(default)]
    entry: Vec<ReferenceEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ReferenceMeta {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
}

/// A single code reference. `message` states what the code means;
/// `source` is an optional verification link. Reference, not advice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceEntry {
    /// Tool the entry belongs to (filled from `meta.name` at load time).
    pub tool: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Loaded code → meaning lookups. Keyed by code; one code can map to
/// several tools (docker 130 and aws 130 are different things).
#[derive(Debug, Default, Clone)]
pub struct ReferenceTable {
    by_code: HashMap<String, Vec<ReferenceEntry>>,
}

impl ReferenceTable {
    /// Load builtin tables plus user overrides from `~/.arshy/reference`.
    pub fn load() -> Result<Self> {
        let mut files: HashMap<String, ReferenceFile> = HashMap::new();

        for (_filename, file) in load_builtins() {
            files.insert(file.meta.name.clone(), file);
        }

        // User overrides: same `meta.name` replaces the builtin.
        let user_dir = crate::config::expand_path(Path::new("${HOME}/.arshy/reference"));
        if let Ok(entries) = std::fs::read_dir(&user_dir) {
            let mut names: Vec<String> = entries
                .flatten()
                .filter_map(|f| f.path().file_name().map(|n| n.to_string_lossy().to_string()))
                .filter(|n| n.ends_with(".toml"))
                .collect();
            names.sort();
            for name in names {
                let path = user_dir.join(&name);
                if let Some(file) = load_from_path(&path) {
                    files.insert(file.meta.name.clone(), file);
                }
            }
        }

        let mut table = Self::default();
        for (tool, file) in files {
            for mut entry in file.entry {
                if entry.tool.is_empty() {
                    entry.tool = tool.clone();
                }
                table.by_code.entry(entry.code.clone()).or_default().push(entry);
            }
        }
        Ok(table)
    }

    /// Look up reference entries for a code, optionally scoped to one tool
    /// (the task's parser name). When a tool is given, entries from other
    /// tools are never returned — docker 130 and aws 130 mean different
    /// things, so mixing them would be wrong.
    pub fn lookup(&self, code: &str, tool: Option<&str>) -> Option<Vec<ReferenceEntry>> {
        let entries = self.by_code.get(code)?;
        match tool {
            Some(t) => {
                let filtered: Vec<ReferenceEntry> =
                    entries.iter().filter(|e| e.tool == t).cloned().collect();
                if filtered.is_empty() {
                    None
                } else {
                    Some(filtered)
                }
            }
            None => Some(entries.clone()),
        }
    }

    /// Total number of loaded entries (builtin + user).
    pub fn count(&self) -> usize {
        self.by_code.values().map(|v| v.len()).sum()
    }
}

#[derive(rust_embed::RustEmbed)]
#[folder = "reference/builtin/"]
#[include = "*.toml"]
struct BuiltinReferenceAssets;

/// Load all builtin reference tables. Logs warnings for parse failures.
fn load_builtins() -> Vec<(String, ReferenceFile)> {
    BuiltinReferenceAssets::iter()
        .filter_map(|file_path| {
            let name = file_path.to_string();
            let file = BuiltinReferenceAssets::get(&file_path)?;
            let content = std::str::from_utf8(file.data.as_ref()).ok()?;
            match toml::from_str::<ReferenceFile>(content) {
                Ok(def) => {
                    if def.meta.name.is_empty() {
                        tracing::warn!("reference file '{}': missing meta.name", name);
                        return None;
                    }
                    Some((name, def))
                }
                Err(e) => {
                    tracing::error!("reference file '{}': {}", name, e);
                    None
                }
            }
        })
        .collect()
}

/// Load a reference table from a filesystem path.
fn load_from_path(path: &Path) -> Option<ReferenceFile> {
    let content = std::fs::read_to_string(path).ok()?;
    match toml::from_str::<ReferenceFile>(&content) {
        Ok(def) => {
            if def.meta.name.is_empty() {
                tracing::warn!("reference file '{}': missing meta.name", path.display());
                return None;
            }
            Some(def)
        }
        Err(e) => {
            tracing::warn!("reference file '{}': {}", path.display(), e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_reference_tables_load() {
        let table = ReferenceTable::load().unwrap();
        assert!(table.count() >= 3, "docker/kubectl/aws tables must load, got {}", table.count());

        // docker exit codes are the canonical non-obvious case.
        let docker = table.lookup("125", None).unwrap();
        assert!(docker.iter().any(|e| e.tool == "docker"));
        assert!(docker.iter().any(|e| e.message.contains("daemon")));
        assert!(docker.iter().all(|e| e.source.is_some()));

        // Same code across tools stays distinguishable (docker 130 vs aws 130).
        let sigint = table.lookup("130", None).unwrap();
        assert!(sigint.iter().any(|e| e.tool == "docker"));
        assert!(sigint.iter().any(|e| e.tool == "aws"));

        // Tool-scoped lookup never mixes meanings.
        let docker_130 = table.lookup("130", Some("docker")).unwrap();
        assert!(docker_130.iter().all(|e| e.tool == "docker"));
        let aws_130 = table.lookup("130", Some("aws")).unwrap();
        assert!(aws_130.iter().all(|e| e.tool == "aws"));
        assert!(table.lookup("130", Some("kubectl")).is_none());
    }

    #[test]
    fn unknown_code_returns_none() {
        let table = ReferenceTable::load().unwrap();
        assert!(table.lookup("999999", None).is_none());
    }
}
