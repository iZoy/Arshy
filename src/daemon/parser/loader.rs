//! File-system watcher for hot-reloading parser changes.
//!
//! Uses polling-based file modification time checking as a fallback.
//! When the `notify` feature is enabled, uses filesystem events instead.
//! Runs in a background thread, checking for changes every `interval` seconds.
//! When a parser file is modified, the callback is invoked to trigger a reload.

#![allow(dead_code)] // entire module dormant until notify crate enabled

use arshy_lib::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// A file watcher for parser directories.
///
/// Polls configured directories for changes to `.toml` and `.rhai` files.
/// When a change is detected, invokes the provided callback.
///
/// Dropping the watcher (or calling `stop()`) stops the background thread.
#[allow(dead_code)] // notify crate unavailable; polling watcher ready for enablement
pub struct ParserWatcher {
    running: Arc<AtomicBool>,
}

impl ParserWatcher {
    /// Start watching the given directories in a background thread.
    /// Calls `on_change` when any parser file is created, modified, or deleted.
    pub fn start<F>(dirs: &[PathBuf], on_change: F) -> Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let dirs = dirs.to_vec();
        let interval = Duration::from_secs(5);

        std::thread::Builder::new()
            .name("parser-watcher".into())
            .spawn(move || {
                let mut mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();

                while running_clone.load(Ordering::Relaxed) {
                    std::thread::sleep(interval);

                    let mut changed = false;
                    let mut current_files = std::collections::HashSet::new();

                    for dir in &dirs {
                        let expanded = arshy_lib::config::expand_path(dir);
                        if !expanded.is_dir() {
                            continue;
                        }
                        if let Ok(entries) = std::fs::read_dir(&expanded) {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if !is_parser_file(&path) {
                                    continue;
                                }
                                current_files.insert(path.clone());

                                if let Ok(meta) = std::fs::metadata(&path) {
                                    if let Ok(mtime) = meta.modified() {
                                        let prev = mtimes.get(&path).copied();
                                        if prev.is_none_or(|p| mtime > p) {
                                            mtimes.insert(path, mtime);
                                            changed = true;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Detect deleted files
                    let deleted: Vec<PathBuf> = mtimes
                        .keys()
                        .filter(|p| !current_files.contains(*p))
                        .cloned()
                        .collect();
                    for p in &deleted {
                        mtimes.remove(p);
                        changed = true;
                    }

                    if changed {
                        tracing::info!("parser files changed, reloading");
                        on_change();
                    }
                }
            })
            .map_err(|e| arshy_lib::ArshyError::Config(format!("failed to start watcher: {}", e)))?;

        Ok(Self { running })
    }

    /// Stop the watcher.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl Drop for ParserWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Check if a file path is a parser definition file.
fn is_parser_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("toml") | Some("rhai")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_parser_file() {
        assert!(is_parser_file(std::path::Path::new("tsc.toml")));
        assert!(is_parser_file(std::path::Path::new("npm.rhai")));
        assert!(!is_parser_file(std::path::Path::new("readme.md")));
        assert!(!is_parser_file(std::path::Path::new("test.txt")));
        assert!(!is_parser_file(std::path::Path::new("noext")));
    }
}
