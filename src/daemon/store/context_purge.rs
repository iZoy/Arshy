//! One-time removal of persisted source-context and Git-correlation metadata.

use crate::Result;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

const MARKER: &str = ".context-purged-v1";

/// Remove contextual fields without changing command output or diagnostics.
///
/// Malformed JSONL rows are retained byte-for-byte and reported. The migration
/// remains incomplete until those rows are repaired, so the next open retries
/// the cleanup instead of silently declaring success.
pub(super) fn purge_legacy_context(store_dir: &Path) -> Result<()> {
    let marker = store_dir.join(MARKER);
    if marker.exists() {
        return Ok(());
    }

    let mut incomplete = false;
    let tasks_path = store_dir.join("tasks.jsonl");
    if tasks_path.exists() {
        incomplete |= scrub_jsonl(&tasks_path, scrub_task_record)?;
    }

    let events_dir = store_dir.join("events");
    if events_dir.exists() {
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&events_dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                paths.push(path);
            }
        }
        paths.sort();
        for path in paths {
            incomplete |= scrub_jsonl(&path, scrub_event)?;
        }
    }

    if incomplete {
        tracing::warn!(
            "context cleanup is incomplete for {}; malformed JSONL rows were preserved",
            store_dir.display()
        );
        return Ok(());
    }

    let mut file = super::create_private_file(&marker)?;
    file.write_all(b"context-purged-v1\n")?;
    file.sync_all()?;
    Ok(())
}

/// Return true if malformed rows were encountered.
fn scrub_jsonl(path: &Path, scrub: fn(&mut Value)) -> Result<bool> {
    let content = std::fs::read_to_string(path)?;
    let mut rewritten = String::with_capacity(content.len());
    let mut changed = false;
    let mut incomplete = false;

    for (index, line) in content.split_inclusive('\n').enumerate() {
        let body = line.strip_suffix('\n').unwrap_or(line);
        if body.trim().is_empty() {
            rewritten.push_str(line);
            continue;
        }
        match serde_json::from_str::<Value>(body) {
            Ok(mut value) => {
                let original = value.clone();
                scrub(&mut value);
                if value != original {
                    changed = true;
                    rewritten.push_str(&serde_json::to_string(&value)?);
                    if line.ends_with('\n') {
                        rewritten.push('\n');
                    }
                } else {
                    rewritten.push_str(line);
                }
            }
            Err(error) => {
                incomplete = true;
                tracing::warn!(
                    "preserving malformed store row in {} at line {} during context cleanup: {}",
                    path.display(),
                    index + 1,
                    error
                );
                rewritten.push_str(line);
            }
        }
    }

    if changed {
        replace_file_atomically(path, rewritten.as_bytes())?;
    }
    Ok(incomplete)
}

fn scrub_task_record(value: &mut Value) {
    if let Some(record) = value.as_object_mut() {
        record.remove("correlated_errors");
        if let Some(metrics) = record.get_mut("metrics").and_then(Value::as_object_mut) {
            metrics.remove("contexts_enriched");
        }
    }
}

fn scrub_event(value: &mut Value) {
    if let Some(event) = value.as_object_mut() {
        event.remove("context");
    }
}

fn replace_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = temporary_path(path);
    let mut file = super::create_private_file(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".context-purge-{}.tmp", uuid::Uuid::new_v4()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migration_removes_context_and_counters_once_while_preserving_other_data() {
        let temp = TempDir::new().unwrap();
        let store = temp.path();
        std::fs::create_dir(store.join("events")).unwrap();
        std::fs::create_dir(store.join("raw")).unwrap();
        std::fs::write(store.join("raw/t1.txt"), "raw bytes remain").unwrap();
        let task_path = store.join("tasks.jsonl");
        std::fs::write(
            &task_path,
            "{\"task_id\":\"t1\",\"correlated_errors\":3,\"raw_output\":\"original command output\",\"metrics\":{\"contexts_enriched\":2,\"raw_output_bytes\":17}}\n",
        )
        .unwrap();
        let event_path = store.join("events/t1.jsonl");
        std::fs::write(
            &event_path,
            "{\"seq\":1,\"message\":\"error E1\",\"context\":{\"line\":\"secret source\"}}\n",
        )
        .unwrap();

        purge_legacy_context(store).unwrap();
        let task: Value =
            serde_json::from_str(std::fs::read_to_string(task_path).unwrap().trim()).unwrap();
        let event: Value =
            serde_json::from_str(std::fs::read_to_string(event_path).unwrap().trim()).unwrap();
        assert_eq!(task["task_id"], "t1");
        assert_eq!(task["raw_output"], "original command output");
        assert_eq!(task["metrics"]["raw_output_bytes"], 17);
        assert!(task.get("correlated_errors").is_none());
        assert!(task["metrics"].get("contexts_enriched").is_none());
        assert_eq!(event["message"], "error E1");
        assert!(event.get("context").is_none());
        assert_eq!(std::fs::read_to_string(store.join("raw/t1.txt")).unwrap(), "raw bytes remain");

        purge_legacy_context(store).unwrap();
        assert_eq!(std::fs::read_to_string(store.join(MARKER)).unwrap(), "context-purged-v1\n");
    }

    #[test]
    fn malformed_rows_are_preserved_and_do_not_mark_migration_complete() {
        let temp = TempDir::new().unwrap();
        let store = temp.path();
        std::fs::create_dir(store.join("events")).unwrap();
        let event_path = store.join("events/t1.jsonl");
        let original = b"not-json\n{\"seq\":1,\"context\":{}}\n";
        std::fs::write(&event_path, original).unwrap();

        purge_legacy_context(store).unwrap();
        let rows = std::fs::read_to_string(event_path).unwrap();
        assert!(rows.starts_with("not-json\n"));
        assert!(!rows.contains("context"));
        assert!(!store.join(MARKER).exists());
    }
}
