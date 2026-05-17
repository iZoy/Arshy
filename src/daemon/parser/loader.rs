//! File-system watcher for hot-reloading parser changes.
//!
//! Uses `notify` v7 for filesystem event monitoring.
//! When a `.toml` or `.rhai` parser file is created, modified, or deleted,
//! invokes the callback to trigger a registry reload.

use arshy_lib::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// A file watcher for parser directories.
///
/// Watches configured directories for changes to `.toml` and `.rhai` files.
/// When a change is detected, invokes the provided callback.
///
/// Dropping the watcher stops the background thread.
pub struct ParserWatcher {
    _watcher: RecommendedWatcher,
}

impl ParserWatcher {
    /// Start watching the given directories for parser file changes.
    /// Calls `on_change` when any parser file is created, modified, or deleted.
    pub fn start<F>(dirs: &[PathBuf], on_change: F) -> Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

        let mut watcher = RecommendedWatcher::new(
            tx,
            notify::Config::default().with_poll_interval(Duration::from_secs(2)),
        )
        .map_err(|e| arshy_lib::ArshyError::Config(format!("notify watcher: {}", e)))?;

        // Watch all configured parser directories
        for dir in dirs {
            let expanded = arshy_lib::config::expand_path(dir);
            if expanded.is_dir() {
                watcher.watch(&expanded, RecursiveMode::NonRecursive).map_err(|e| {
                    arshy_lib::ArshyError::Config(format!("watch {:?}: {}", expanded, e))
                })?;
                tracing::debug!("watching parser dir: {}", expanded.display());
            }
        }

        // Background thread to receive events and debounce
        std::thread::Builder::new()
            .name("parser-watcher".into())
            .spawn(move || {
                let mut last_trigger = std::time::Instant::now();
                let debounce = Duration::from_millis(500);

                while let Ok(event_result) = rx.recv() {
                    match event_result {
                        Ok(event) => {
                            if is_parser_event(&event) && last_trigger.elapsed() > debounce {
                                last_trigger = std::time::Instant::now();
                                tracing::info!("parser file changed: {:?}", event.paths);
                                on_change();
                            }
                        }
                        Err(e) => {
                            tracing::warn!("watch error: {}", e);
                        }
                    }
                }
            })
            .map_err(|e| arshy_lib::ArshyError::Config(format!("watcher thread: {}", e)))?;

        Ok(Self { _watcher: watcher })
    }
}

/// Check if a notify event involves parser files.
fn is_parser_event(event: &Event) -> bool {
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => event
            .paths
            .iter()
            .any(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("toml") | Some("rhai"))),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_parser_event_create() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/tmp/test.toml")],
            attrs: Default::default(),
        };
        assert!(is_parser_event(&event));
    }

    #[test]
    fn test_is_parser_event_rhai() {
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/tmp/custom.rhai")],
            attrs: Default::default(),
        };
        assert!(is_parser_event(&event));
    }

    #[test]
    fn test_is_parser_event_non_parser() {
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/tmp/readme.md")],
            attrs: Default::default(),
        };
        assert!(!is_parser_event(&event));
    }

    #[test]
    fn test_is_parser_event_mixed_paths() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/tmp/data.txt"), PathBuf::from("/tmp/parser.toml")],
            attrs: Default::default(),
        };
        assert!(is_parser_event(&event));
    }

    #[test]
    fn test_is_parser_event_remove() {
        let event = Event {
            kind: EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![PathBuf::from("/tmp/deleted.rhai")],
            attrs: Default::default(),
        };
        assert!(is_parser_event(&event));
    }
}
