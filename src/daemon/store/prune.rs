use crate::Result;

impl super::Store {
    /// Keep only the `keep` most recent tasks, delete the rest.
    /// Returns (tasks_deleted, events_deleted).
    pub fn prune_keep(&self, keep: usize) -> Result<(usize, usize)> {
        let mut tasks = self.lock();
        let open_event_tasks = self
            .event_files
            .lock()
            .map_err(|_| crate::ArshyError::Other("event file mutex poisoned".into()))?;

        if tasks.is_empty() {
            return Ok((0, 0));
        }

        // Edge case: keep=0 means "delete everything"
        if keep == 0 {
            let to_delete: Vec<String> = tasks
                .iter()
                .filter(|(id, record)| {
                    record.task.status != crate::ipc::TaskStatus::Running
                        && !open_event_tasks.contains_key(*id)
                })
                .map(|(id, _)| id.clone())
                .collect();
            let de = count_event_files(&self.dir, to_delete.iter())?;
            let removed: Vec<(String, super::TaskRecord)> = to_delete
                .iter()
                .filter_map(|id| tasks.remove(id).map(|record| (id.clone(), record)))
                .collect();
            let dt = removed.len();
            if let Err(error) = self.persist_tasks_snapshot(&tasks) {
                tasks.extend(removed);
                self.mark_dirty();
                return Err(error);
            }
            remove_event_files(&self.dir, to_delete.iter())?;
            remove_raw_files(&self.dir, to_delete.iter())?;
            return Ok((dt, de));
        }

        // Active tasks cannot be pruned: their event append handles may still
        // be open, and unlinking the file would make subsequent events vanish.
        let mut sorted: Vec<(&String, &super::TaskRecord)> = tasks
            .iter()
            .filter(|(id, record)| {
                record.task.status != crate::ipc::TaskStatus::Running
                    && !open_event_tasks.contains_key(*id)
            })
            .collect();
        sorted.sort_by(|a, b| b.1.task.started_at.cmp(&a.1.task.started_at));

        if sorted.len() <= keep {
            return Ok((0, 0));
        }

        // Everything from index keep onwards needs to be deleted
        let to_delete: Vec<String> = sorted[keep..].iter().map(|(id, _)| (*id).clone()).collect();
        let de = count_event_files(&self.dir, to_delete.iter())?;
        let removed: Vec<(String, super::TaskRecord)> =
            to_delete.iter().filter_map(|id| tasks.remove(id).map(|r| (id.clone(), r))).collect();
        let dt = removed.len();
        if let Err(error) = self.persist_tasks_snapshot(&tasks) {
            tasks.extend(removed);
            self.mark_dirty();
            return Err(error);
        }
        remove_event_files(&self.dir, to_delete.iter())?;
        remove_raw_files(&self.dir, to_delete.iter())?;

        Ok((dt, de))
    }

    /// Delete tasks older than `days` days. Returns (tasks_deleted, events_deleted).
    pub fn prune_older_than(&self, days: u32) -> Result<(usize, usize)> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();

        let mut tasks = self.lock();
        let open_event_tasks = self
            .event_files
            .lock()
            .map_err(|_| crate::ArshyError::Other("event file mutex poisoned".into()))?;

        let to_delete: Vec<String> = tasks
            .iter()
            .filter(|(_, r)| {
                r.task.status != crate::ipc::TaskStatus::Running
                    && !open_event_tasks.contains_key(r.task.task_id.as_str())
                    && r.task.started_at < cutoff
            })
            .map(|(id, _)| id.clone())
            .collect();

        let de = count_event_files(&self.dir, to_delete.iter())?;
        let removed: Vec<(String, super::TaskRecord)> =
            to_delete.iter().filter_map(|id| tasks.remove(id).map(|r| (id.clone(), r))).collect();
        let dt = removed.len();
        if dt > 0 {
            if let Err(error) = self.persist_tasks_snapshot(&tasks) {
                tasks.extend(removed);
                self.mark_dirty();
                return Err(error);
            }
            remove_event_files(&self.dir, to_delete.iter())?;
            remove_raw_files(&self.dir, to_delete.iter())?;
        }

        Ok((dt, de))
    }
}

/// Count total event lines across all event files for the given task IDs.
fn count_all_events<'a>(
    dir: &std::path::Path,
    task_ids: impl Iterator<Item = &'a String>,
) -> Result<usize> {
    let events_dir = dir.join("events");
    let mut total = 0;
    for id in task_ids {
        let path = events_dir.join(format!("{}.jsonl", id));
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            total += content.lines().filter(|l| !l.trim().is_empty()).count();
        }
    }
    Ok(total)
}

/// Count event lines for a subset of task IDs.
fn count_event_files<'a>(
    dir: &std::path::Path,
    task_ids: impl Iterator<Item = &'a String>,
) -> Result<usize> {
    count_all_events(dir, task_ids)
}

/// Remove event JSONL files for the given task IDs.
fn remove_event_files<'a>(
    dir: &std::path::Path,
    task_ids: impl Iterator<Item = &'a String>,
) -> Result<()> {
    let events_dir = dir.join("events");
    for id in task_ids {
        let path = events_dir.join(format!("{}.jsonl", id));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Remove raw output files for the given task IDs.
fn remove_raw_files<'a>(
    dir: &std::path::Path,
    task_ids: impl Iterator<Item = &'a String>,
) -> Result<()> {
    let raw_dir = dir.join("raw");
    for id in task_ids {
        let path = raw_dir.join(format!("{}.txt", id));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Clean up stale symlink fallback directories in /tmp/.arshy-cwd/.
/// Removes symlinks older than 1 hour that may be left over from crashed tasks.
pub fn cleanup_stale_symlinks() {
    // SAFETY: geteuid has no preconditions and only reads the effective uid.
    let uid = unsafe { libc::geteuid() };
    let legacy_dir = std::path::Path::new("/tmp/.arshy-cwd");
    let current_dir = std::path::PathBuf::from(format!("/tmp/.arshy-cwd-{uid}"));
    cleanup_stale_symlink_layout(legacy_dir, &current_dir, uid);
}

fn cleanup_stale_symlink_layout(
    legacy_dir: &std::path::Path,
    current_dir: &std::path::Path,
    uid: u32,
) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if let Ok(directory) = std::fs::symlink_metadata(legacy_dir) {
        // The legacy directory could have been pre-created in shared /tmp.
        // Only inspect it when owned by this UID and not writable by others.
        if directory.file_type().is_dir()
            && directory.uid() == uid
            && directory.permissions().mode() & 0o022 == 0
        {
            if let Ok(entries) = std::fs::read_dir(legacy_dir) {
                let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                        continue;
                    };
                    if !metadata.file_type().is_symlink() || metadata.uid() != uid {
                        continue;
                    }
                    let stale =
                        metadata.modified().map(|modified| modified < cutoff).unwrap_or(true);
                    if stale
                        || !std::fs::metadata(&path).map(|target| target.is_dir()).unwrap_or(false)
                    {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
            let _ = std::fs::remove_dir(legacy_dir);
        }
    }

    // The current layout uses a private per-UID directory. Remove crash
    // leftovers there as well; old cleanup above only knows the legacy path.
    cleanup_stale_symlinks_in(current_dir, uid);
}

fn cleanup_stale_symlinks_in(symlink_dir: &std::path::Path, uid: u32) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let Ok(directory) = std::fs::symlink_metadata(symlink_dir) else {
        return;
    };
    // Do not enumerate a replaced path or a directory writable by other UIDs.
    if !directory.file_type().is_dir()
        || directory.uid() != uid
        || directory.permissions().mode() & 0o077 != 0
    {
        return;
    }

    let Ok(entries) = std::fs::read_dir(symlink_dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_symlink() || metadata.uid() != uid {
            continue;
        }

        let stale = metadata.modified().map(|modified| modified < cutoff).unwrap_or(true);
        let target_is_directory =
            std::fs::metadata(&path).map(|target| target.is_dir()).unwrap_or(false);
        if stale || !target_is_directory {
            let _ = std::fs::remove_file(path);
        }
    }
    let _ = std::fs::remove_dir(symlink_dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::store::Store;
    use crate::ipc::{Task, TaskEvent, TaskStatus};
    use tempfile::TempDir;

    fn test_store() -> (Store, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        store.initialize_schema().unwrap();
        (store, tmp)
    }

    fn make_task(id: &str, started_at: &str, status: TaskStatus) -> Task {
        Task {
            task_id: id.to_string(),
            command: format!("echo {id}"),
            cwd: Some("/tmp".to_string()),
            status,
            exit_code: None,
            pid: None,
            parser_name: None,
            started_at: started_at.to_string(),
            finished_at: None,
            duration_ms: None,
            events_count: 0,
            error_count: 0,
            purpose: None,
            carrier: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_cwd_cleanup_removes_broken_links_but_keeps_valid_links() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let tmp = TempDir::new().unwrap();
        let directory = tmp.path().join("cwd-links");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let target = tmp.path().join("project");
        std::fs::create_dir(&target).unwrap();
        let valid_link = directory.join("valid");
        let broken_link = directory.join("broken");
        let regular_file = directory.join("keep.txt");
        symlink(&target, &valid_link).unwrap();
        symlink(tmp.path().join("missing"), &broken_link).unwrap();
        std::fs::write(&regular_file, "keep").unwrap();
        // SAFETY: geteuid has no preconditions and only reads the effective uid.
        let uid = unsafe { libc::geteuid() };

        cleanup_stale_symlinks_in(&directory, uid);

        assert!(std::fs::symlink_metadata(&valid_link).is_ok());
        assert!(std::fs::symlink_metadata(&broken_link).is_err());
        assert_eq!(std::fs::read_to_string(regular_file).unwrap(), "keep");
        assert!(directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn private_cwd_cleanup_skips_non_private_directories() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let tmp = TempDir::new().unwrap();
        let directory = tmp.path().join("shared-cwd-links");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        let broken_link = directory.join("broken");
        symlink(tmp.path().join("missing"), &broken_link).unwrap();
        // SAFETY: geteuid has no preconditions and only reads the effective uid.
        let uid = unsafe { libc::geteuid() };

        cleanup_stale_symlinks_in(&directory, uid);

        assert!(std::fs::symlink_metadata(&broken_link).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn cwd_cleanup_checks_new_layout_even_when_legacy_directory_is_missing() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy-missing");
        let current = tmp.path().join("private-current");
        std::fs::create_dir(&current).unwrap();
        std::fs::set_permissions(&current, std::fs::Permissions::from_mode(0o700)).unwrap();
        let broken_link = current.join("broken");
        symlink(tmp.path().join("missing"), &broken_link).unwrap();
        // SAFETY: geteuid has no preconditions and only reads the effective uid.
        let uid = unsafe { libc::geteuid() };

        cleanup_stale_symlink_layout(&legacy, &current, uid);

        assert!(std::fs::symlink_metadata(&broken_link).is_err());
    }

    #[test]
    fn prune_keep_retains_most_recent() {
        let (store, _t) = test_store();
        store
            .insert_task(&make_task("old1", "2024-01-01T00:00:00Z", TaskStatus::Completed))
            .unwrap();
        store
            .insert_task(&make_task("old2", "2024-06-01T00:00:00Z", TaskStatus::Completed))
            .unwrap();
        store
            .insert_task(&make_task("recent", "2025-01-01T00:00:00Z", TaskStatus::Completed))
            .unwrap();

        let (deleted, events) = store.prune_keep(1).unwrap();
        assert_eq!(deleted, 2);
        assert_eq!(events, 0);
        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].task_id, "recent");
    }

    #[test]
    fn prune_keep_zero_deletes_all() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("a", "2024-01-01T00:00:00Z", TaskStatus::Completed)).unwrap();
        store.insert_task(&make_task("b", "2024-02-01T00:00:00Z", TaskStatus::Completed)).unwrap();
        let (deleted, _) = store.prune_keep(0).unwrap();
        assert_eq!(deleted, 2);
        assert!(store.list_tasks(None, 100).unwrap().is_empty());
    }

    #[test]
    fn prune_never_removes_running_tasks() {
        let (store, _t) = test_store();
        store
            .insert_task(&make_task("active", "2020-01-01T00:00:00Z", TaskStatus::Running))
            .unwrap();
        store
            .insert_task(&make_task("finished", "2020-01-01T00:00:00Z", TaskStatus::Completed))
            .unwrap();

        let (deleted_by_age, _) = store.prune_older_than(1).unwrap();
        assert_eq!(deleted_by_age, 1);
        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].task_id, "active");

        let (deleted_by_count, _) = store.prune_keep(0).unwrap();
        assert_eq!(deleted_by_count, 0);
        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].task_id, "active");
    }

    #[test]
    fn prune_preserves_terminal_task_while_event_stream_is_open() {
        let (store, _t) = test_store();
        store
            .insert_task(&make_task("enriching", "2020-01-01T00:00:00Z", TaskStatus::Running))
            .unwrap();
        store
            .insert_event(
                "enriching",
                1,
                &TaskEvent {
                    seq: 1,
                    event_type: "diagnostic".into(),
                    severity: Some("error".into()),
                    code: None,
                    message: "still enriching".into(),
                    location: None,
                    context: None,
                    hint: None,
                },
            )
            .unwrap();
        store.update_task("enriching", &TaskStatus::Completed, Some(1), Some(1)).unwrap();

        let (deleted, _) = store.prune_keep(0).unwrap();
        assert_eq!(deleted, 0);
        assert!(store.get_task("enriching").unwrap().is_some());
        assert!(store.store_dir().join("events/enriching.jsonl").exists());
    }

    #[test]
    fn prune_and_event_append_do_not_deadlock() {
        use std::sync::{mpsc, Arc};
        use std::time::{Duration, Instant};

        let (store, _tmp) = test_store();
        let store = Arc::new(store);
        let task_id = "prune-race";
        store
            .insert_task(&make_task(task_id, "2020-01-01T00:00:00Z", TaskStatus::Completed))
            .unwrap();

        // Hold the event mutex so prune acquires the task mutex and waits for
        // the event mutex. Start an append behind prune, then release the event
        // mutex to force both operations through the same lock-order window.
        let event_guard = store.event_files.lock().unwrap();
        let (prune_done_tx, prune_done_rx) = mpsc::channel();
        let prune_store = Arc::clone(&store);
        std::thread::spawn(move || {
            let _ = prune_done_tx.send(prune_store.prune_keep(0));
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match store.tasks.try_lock() {
                Ok(tasks) => drop(tasks),
                Err(std::sync::TryLockError::WouldBlock) => break,
                Err(std::sync::TryLockError::Poisoned(_)) => panic!("task mutex poisoned"),
            }
            assert!(Instant::now() < deadline, "prune did not acquire the task mutex");
            std::thread::yield_now();
        }

        let (append_started_tx, append_started_rx) = mpsc::channel();
        let (append_done_tx, append_done_rx) = mpsc::channel();
        let append_store = Arc::clone(&store);
        std::thread::spawn(move || {
            append_started_tx.send(()).unwrap();
            let event = TaskEvent {
                seq: 1,
                event_type: "log".into(),
                severity: None,
                code: None,
                message: "concurrent append".into(),
                location: None,
                context: None,
                hint: None,
            };
            let _ = append_done_tx.send(append_store.insert_event(task_id, 1, &event));
        });
        append_started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(Duration::from_millis(10));
        drop(event_guard);

        let pruned = prune_done_rx.recv_timeout(Duration::from_secs(2));
        assert!(pruned.is_ok(), "prune deadlocked with a concurrent event append");
        pruned.unwrap().unwrap();
        let append_result = append_done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("event append remained blocked after prune completed");
        let task_remains = store.get_task(task_id).unwrap().is_some();
        let event_file_remains = store.store_dir().join("events/prune-race.jsonl").exists();
        if task_remains {
            append_result.unwrap();
            assert!(event_file_remains, "prune removed an event stream that append opened");
        } else {
            assert!(append_result.is_err(), "append to a pruned task should be rejected");
            assert!(!event_file_remains, "append created an orphan event file after prune");
        }
    }

    #[test]
    fn prune_keep_noop_when_under_limit() {
        let (store, _t) = test_store();
        store.insert_task(&make_task("a", "2024-01-01T00:00:00Z", TaskStatus::Completed)).unwrap();
        let (deleted, _) = store.prune_keep(10).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn prune_older_than_removes_stale_tasks() {
        let (store, _t) = test_store();
        store
            .insert_task(&make_task("ancient", "2020-01-01T00:00:00Z", TaskStatus::Completed))
            .unwrap();
        store
            .insert_task(&make_task(
                "fresh",
                &chrono::Utc::now().to_rfc3339(),
                TaskStatus::Completed,
            ))
            .unwrap();

        let (deleted, _) = store.prune_older_than(1).unwrap();
        assert_eq!(deleted, 1);
        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].task_id, "fresh");
    }

    #[test]
    fn cleanup_stale_symlinks_does_not_panic() {
        // Touches the global /tmp/.arshy-cwd cleanup path; must not panic.
        cleanup_stale_symlinks();
    }
}
