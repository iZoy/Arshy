use crate::Result;

impl super::Store {
    /// Keep only the `keep` most recent tasks, delete the rest.
    /// Returns (tasks_deleted, events_deleted).
    pub fn prune_keep(&self, keep: usize) -> Result<(usize, usize)> {
        let mut tasks = self.lock();

        if tasks.is_empty() {
            return Ok((0, 0));
        }

        // Edge case: keep=0 means "delete everything"
        if keep == 0 {
            let dt = tasks.len();
            let de = count_all_events(&self.dir, tasks.keys())?;
            // Remove event files and raw output files
            remove_event_files(&self.dir, tasks.keys())?;
            remove_raw_files(&self.dir, tasks.keys())?;
            tasks.clear();
            drop(tasks);
            self.persist_tasks()?;
            return Ok((dt, de));
        }

        // Sort by started_at DESC to find the cutoff
        let mut sorted: Vec<(&String, &super::TaskRecord)> = tasks.iter().collect();
        sorted.sort_by(|a, b| b.1.task.started_at.cmp(&a.1.task.started_at));

        if sorted.len() <= keep {
            return Ok((0, 0));
        }

        // Everything from index keep onwards needs to be deleted
        let to_delete: Vec<String> = sorted[keep..].iter().map(|(id, _)| (*id).clone()).collect();
        let dt = to_delete.len();
        let de = count_event_files(&self.dir, to_delete.iter())?;

        // Remove event files and raw output files
        remove_event_files(&self.dir, to_delete.iter())?;
        remove_raw_files(&self.dir, to_delete.iter())?;

        // Remove from HashMap
        for id in &to_delete {
            tasks.remove(id);
        }
        drop(tasks);
        self.persist_tasks()?;

        Ok((dt, de))
    }

    /// Delete tasks older than `days` days. Returns (tasks_deleted, events_deleted).
    pub fn prune_older_than(&self, days: u32) -> Result<(usize, usize)> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();

        let mut tasks = self.lock();

        let to_delete: Vec<String> = tasks
            .iter()
            .filter(|(_, r)| r.task.started_at < cutoff)
            .map(|(id, _)| id.clone())
            .collect();

        let dt = to_delete.len();
        let de = count_event_files(&self.dir, to_delete.iter())?;

        // Remove event files and raw output files
        remove_event_files(&self.dir, to_delete.iter())?;
        remove_raw_files(&self.dir, to_delete.iter())?;

        // Remove from HashMap
        for id in &to_delete {
            tasks.remove(id);
        }
        drop(tasks);
        if dt > 0 {
            self.persist_tasks()?;
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
    let symlink_dir = std::path::Path::new("/tmp/.arshy-cwd");
    let Ok(entries) = std::fs::read_dir(symlink_dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    for entry in entries.flatten() {
        let path = entry.path();
        // Only remove symlinks, not regular files
        if !path.is_symlink() {
            continue;
        }
        // Remove if older than cutoff or if target no longer exists
        let stale =
            path.symlink_metadata().and_then(|m| m.modified()).map(|t| t < cutoff).unwrap_or(true);
        if stale || !std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false) {
            let _ = std::fs::remove_file(&path);
        }
    }
    // Remove the directory itself if empty
    let _ = std::fs::remove_dir(symlink_dir);
}
