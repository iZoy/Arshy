//! File-system watcher for hot-reloading parser changes.
//!
//! Uses polling-based file modification time checking.
//! Runs in a background thread, checking for changes every `interval` seconds.
//! When a parser file is modified, the registry is reloaded.

use arshy_lib::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// A polling-based file watcher for parser directories.
pub struct ParserWatcher {
    dirs: Vec<PathBuf>,
    interval: Duration,
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl ParserWatcher {
    /// Create a watcher for the given directories.
    pub fn new(dirs: &[PathBuf]) -> Result<Self> {
        Ok(Self {
            dirs: dirs.to_vec(),
            interval: Duration::from_secs(5),
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Start watching in a background thread. Calls `on_change` when any parser file changes.
    pub fn start<F>(&self, on_change: F) -> Result<()>
    where
        F: Fn() + Send + 'static,
    {
        self.running.store(true, std::sync::atomic::Ordering::SeqCst);
        let running = self.running.clone();
        let dirs = self.dirs.clone();
        let interval = self.interval;

        std::thread::spawn(move || {
            let mut mtimes: HashMap<PathBuf, SystemTime> = HashMap::new();

            while running.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(interval);

                let mut changed = false;
                for dir in &dirs {
                    if !dir.is_dir() { continue; }
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if !is_parser_file(&path) { continue; }
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

                if changed {
                    tracing::info!("parser files changed, reloading");
                    on_change();
                }
            }
        });

        Ok(())
    }

    /// Stop the watcher.
    pub fn stop(&self) {
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

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
    }
}
