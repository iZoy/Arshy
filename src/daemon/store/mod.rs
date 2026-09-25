//! JSONL file-based storage — tasks and events.

mod context_purge;
mod events;
pub mod prune;
mod schema;
mod tasks;

use crate::ipc::TaskStatus;
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Per-task metrics tracking parser pipeline throughput.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskMetrics {
    pub raw_output_bytes: u64,
    pub structured_events_bytes: u64,
    pub visible_events: u64,
    pub skipped_noise_events: u64,
    pub locations_extracted: u64,
    pub codes_extracted: u64,
    pub pairs_merged: u64,
}

/// Counts for the complete filtered event set, before pagination.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventSummary {
    pub total: u64,
    pub errors: u64,
    pub warnings: u64,
}

/// Internal task record extending the public Task with storage-only fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct TaskRecord {
    #[serde(flatten)]
    task: crate::ipc::Task,
    raw_output: Option<String>,
    dedup_collapsed: u64,
    metrics: TaskMetrics,
}

impl Default for TaskRecord {
    fn default() -> Self {
        Self {
            task: crate::ipc::Task {
                task_id: String::new(),
                command: String::new(),
                cwd: None,
                status: TaskStatus::Running,
                exit_code: None,
                pid: None,
                parser_name: None,
                started_at: String::new(),
                finished_at: None,
                duration_ms: None,
                events_count: 0,
                error_count: 0,
                purpose: None,
                carrier: None,
            },
            raw_output: None,
            dedup_collapsed: 0,
            metrics: TaskMetrics::default(),
        }
    }
}

/// Thread-safe JSONL file-based store.
///
/// Scale trigger (decision 4): JSONL is intentional — append-only, zero
/// deps, debuggable. If tasks exceed ~10k or cross-task query latency
/// exceeds ~200ms, add a derived index (e.g. file → task_id) before
/// considering a DB migration; no second reversal without measurement.
pub struct Store {
    dir: PathBuf,
    tasks: Mutex<HashMap<String, TaskRecord>>,
    /// Malformed task rows retained verbatim so a later snapshot flush does
    /// not destroy recoverable bytes before an operator repairs the store.
    corrupt_task_lines: Mutex<Vec<(usize, String)>>,
    /// Open append handles for active task event streams. Keeping one handle
    /// per task avoids reopening and flushing the JSONL file for every parser
    /// event during a build; handles are dropped when a task is rewritten or
    /// the store is dropped.
    event_files: Mutex<HashMap<String, Option<std::fs::File>>>,
    dirty: AtomicBool,
    /// Epoch seconds of the most recent task activity. Initialised at open
    /// time so a never-used daemon still idle-exits after the timeout, and
    /// refreshed on every execution path so a daemon that only served raw
    /// inspection commands is not treated as idle forever.
    last_activity: AtomicI64,
    /// Number of in-flight fast-path commands. Structured commands are
    /// represented by a persisted `Running` task; fast commands intentionally
    /// skip that allocation, so they need this small in-memory guard to keep
    /// the idle watchdog from exiting during a long read.
    active_fast_commands: AtomicUsize,
    /// Notified whenever `mark_dirty` is called, so the background flush
    /// task can sleep indefinitely while idle instead of polling on a fixed
    /// interval (a 1 Hz poll would needlessly wake the CPU and hurt laptop
    /// battery life).
    flush_notify: Arc<Notify>,
    /// Wakes the daemon's idle deadline whenever execution activity changes.
    /// Kept separate from persistence notifications so neither consumer can
    /// steal the other's permit.
    activity_notify: Arc<Notify>,
}

trait RollbackAppend: std::io::Write {
    fn current_len(&self) -> std::io::Result<u64>;
    fn rollback_to(&mut self, len: u64) -> std::io::Result<()>;
}

impl RollbackAppend for std::fs::File {
    fn current_len(&self) -> std::io::Result<u64> {
        self.metadata().map(|metadata| metadata.len())
    }

    fn rollback_to(&mut self, len: u64) -> std::io::Result<()> {
        self.set_len(len)?;
        self.sync_data()
    }
}

#[derive(Debug)]
struct AppendFailure {
    write_error: std::io::Error,
    rollback_error: Option<std::io::Error>,
}

fn append_jsonl_line<W: RollbackAppend>(
    file: &mut W,
    line: &str,
) -> std::result::Result<(), AppendFailure> {
    let original_len = file
        .current_len()
        .map_err(|write_error| AppendFailure { write_error, rollback_error: None })?;
    let write_result = file.write_all(line.as_bytes()).and_then(|()| file.write_all(b"\n"));
    match write_result {
        Ok(()) => Ok(()),
        Err(write_error) => {
            let rollback_error = file.rollback_to(original_len).err();
            Err(AppendFailure { write_error, rollback_error })
        }
    }
}

fn handle_append_failure(
    files: &mut HashMap<String, Option<std::fs::File>>,
    task_id: &str,
    failure: AppendFailure,
) -> crate::Result<()> {
    if let Some(rollback_error) = failure.rollback_error {
        // Do not append more bytes after a rollback failure: that would make
        // later records appear inside a corrupt JSON row.
        files.insert(task_id.to_string(), None);
        return Err(crate::ArshyError::Other(format!(
            "event append failed for task {task_id}: {}; rollback also failed: {}",
            failure.write_error, rollback_error
        )));
    }
    Err(failure.write_error.into())
}

/// Keep command history, diagnostics, and raw program output private to the
/// current user. Store files can contain credentials copied into commands or
/// printed by build tools, so the store must not inherit a permissive umask.
fn ensure_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    // Open without following symlinks, then harden through the descriptor so
    // a path swap cannot redirect chmod to a different directory.
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = directory.metadata()?;
    // SAFETY: geteuid has no preconditions and only reads the effective uid.
    if !metadata.file_type().is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(crate::ArshyError::Other(format!(
            "refusing unsafe store directory {} (expected a real directory owned by this user)",
            path.display()
        )));
    }
    directory.set_permissions(std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn secure_existing_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            return Err(crate::ArshyError::Other(format!(
                "refusing non-regular store file {}",
                path.display()
            )))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    // Use O_NOFOLLOW and descriptor-based chmod to avoid symlink replacement
    // races between metadata checks and permission changes.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions and only reads the effective uid.
    if !metadata.file_type().is_file() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(crate::ArshyError::Other(format!(
            "refusing unsafe store file {} (expected a regular file owned by this user)",
            path.display()
        )));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Create or replace a private store file, refusing symlink targets.
pub(super) fn create_private_file(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions and only reads the effective uid.
    if !metadata.file_type().is_file() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(crate::ArshyError::Other(format!(
            "refusing unsafe store file {} (expected a regular file owned by this user)",
            path.display()
        )));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn open_private_append(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions and only reads the effective uid.
    if !metadata.file_type().is_file() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(crate::ArshyError::Other(format!(
            "refusing unsafe store file {} (expected a regular file owned by this user)",
            path.display()
        )));
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn secure_store_layout(path: &Path) -> Result<()> {
    ensure_private_dir(path)?;
    let raw_dir = path.join("raw");
    let events_dir = path.join("events");
    ensure_private_dir(&raw_dir)?;
    ensure_private_dir(&events_dir)?;

    for name in ["tasks.jsonl", "tasks.jsonl.tmp", "versions.json"] {
        secure_existing_file(&path.join(name))?;
    }
    for directory in [&raw_dir, &events_dir] {
        for entry in std::fs::read_dir(directory)? {
            secure_existing_file(&entry?.path())?;
        }
    }
    Ok(())
}

impl Store {
    /// Open (or create) the JSONL store directory at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let store_dir = path.to_path_buf();

        secure_store_layout(&store_dir)?;
        context_purge::purge_legacy_context(&store_dir)?;

        let (mut tasks_map, corrupt_task_lines) = load_tasks_from_disk(&store_dir)?;
        let recovered_at = chrono::Utc::now().to_rfc3339();
        let mut recovered = 0usize;
        for record in tasks_map.values_mut() {
            if record.task.status == TaskStatus::Running {
                // A persisted running task belongs to a previous daemon
                // process. Its PTY cannot survive/reconnect, so keeping it
                // Running would block idle exit forever after a crash.
                record.task.status = TaskStatus::Failed;
                record.task.pid = None;
                record.task.finished_at = Some(recovered_at.clone());
                recovered += 1;
            }
        }

        let store = Self {
            dir: store_dir,
            tasks: Mutex::new(tasks_map),
            corrupt_task_lines: Mutex::new(corrupt_task_lines),
            event_files: Mutex::new(HashMap::new()),
            dirty: AtomicBool::new(recovered > 0),
            last_activity: AtomicI64::new(chrono::Utc::now().timestamp()),
            active_fast_commands: AtomicUsize::new(0),
            flush_notify: Arc::new(Notify::new()),
            activity_notify: Arc::new(Notify::new()),
        };
        if recovered > 0 {
            tracing::warn!("recovered {} interrupted task(s) from previous daemon", recovered);
            store.persist_tasks()?;
        }
        Ok(store)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, TaskRecord>> {
        self.tasks.lock().expect("store mutex poisoned")
    }

    /// Write the full tasks HashMap to tasks.jsonl atomically.
    fn persist_tasks(&self) -> Result<()> {
        let tasks = self.tasks.lock().unwrap();
        self.persist_tasks_snapshot(&tasks)
    }

    /// Persist a snapshot when the caller already owns the task mutex. This is
    /// needed by prune, which must keep task and event-file locks together so
    /// an append cannot slip between task removal and file cleanup.
    fn persist_tasks_snapshot(&self, tasks: &HashMap<String, TaskRecord>) -> Result<()> {
        let mut corrupt_task_lines = self
            .corrupt_task_lines
            .lock()
            .map_err(|_| crate::ArshyError::Other("corrupt task lines mutex poisoned".into()))?;
        let path = self.dir.join("tasks.jsonl");
        let tmp = self.dir.join("tasks.jsonl.tmp");
        let mut file = create_private_file(&tmp)?;
        for task in tasks.values() {
            serde_json::to_writer(&mut file, task)?;
            file.write_all(b"\n")?;
        }
        let mut relocated_corrupt_lines = Vec::with_capacity(corrupt_task_lines.len());
        for (index, (_, line)) in corrupt_task_lines.iter().enumerate() {
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
            relocated_corrupt_lines.push((tasks.len() + index + 1, line.clone()));
        }
        file.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        *corrupt_task_lines = relocated_corrupt_lines;
        self.dirty.store(false, Ordering::Release);
        Ok(())
    }

    /// Mark the store as having unsaved changes and wake the flush task.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
        self.flush_notify.notify_one();
    }

    /// Seconds since the most recent task activity, or `None` while a task is
    /// still running (the system is clearly not idle).
    ///
    /// Used by the idle-exit watchdog in the daemon accept loop: once this
    /// exceeds `daemon.idle_timeout_secs`, the daemon self-exits. A live
    /// task short-circuits to `None`. Structured tasks mark on start and
    /// completion; raw fast-path commands mark on start, so either path keeps
    /// the deadline correct.
    pub fn idle_since_secs(&self) -> Result<Option<u64>> {
        if self.active_fast_commands.load(Ordering::Acquire) > 0 {
            return Ok(None);
        }
        let tasks = self.lock();
        for record in tasks.values() {
            if matches!(record.task.status, crate::ipc::TaskStatus::Running) {
                return Ok(None);
            }
        }
        let last = self.last_activity.load(Ordering::Relaxed);
        let secs = (chrono::Utc::now().timestamp() - last).max(0) as u64;
        Ok(Some(secs))
    }

    /// Record that a task ran (short or long path). Keeps the idle watchdog's
    /// "time since last activity" fresh for commands that skip the store.
    pub fn mark_activity(&self) {
        self.last_activity.store(chrono::Utc::now().timestamp(), Ordering::Relaxed);
        self.activity_notify.notify_one();
    }

    /// Mark a zero-persistence fast-path command as running.
    pub fn begin_fast_activity(&self) {
        self.active_fast_commands.fetch_add(1, Ordering::AcqRel);
        self.mark_activity();
    }

    /// Mark a zero-persistence fast-path command as finished.
    pub fn end_fast_activity(&self) {
        self.active_fast_commands.fetch_sub(1, Ordering::AcqRel);
        self.mark_activity();
    }

    /// Sleep until the store has been inactive for `timeout_secs`.
    ///
    /// This is event-driven: activity resets one exact deadline, and running
    /// tasks suspend the deadline until their completion notification. An idle
    /// daemon therefore performs no periodic polling or per-second wake-up.
    pub async fn wait_until_idle(&self, timeout_secs: u64) {
        if timeout_secs == 0 {
            std::future::pending::<()>().await;
            return;
        }

        loop {
            // Register before checking state so activity between the check and
            // await cannot be lost (`Notify` retains a permit).
            let activity = self.activity_notify.notified();
            tokio::pin!(activity);

            match self.idle_since_secs() {
                Ok(Some(elapsed)) if elapsed >= timeout_secs => return,
                Ok(Some(elapsed)) => {
                    let remaining = timeout_secs.saturating_sub(elapsed).max(1);
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(remaining)) => {}
                        _ = &mut activity => {}
                    }
                }
                Ok(None) | Err(_) => activity.await,
            }
        }
    }

    /// Flush pending changes to disk if dirty. Returns Ok(()) even if not dirty.
    pub fn flush(&self) -> Result<()> {
        if self.dirty.load(Ordering::Acquire) {
            self.persist_tasks()?;
        }
        Ok(())
    }

    /// Spawn a background task that flushes dirty state to disk.
    ///
    /// Battery-friendly: the task sleeps until `mark_dirty` signals it, so an
    /// idle daemon performs zero periodic wake-ups. `Notify` retains a permit
    /// when no waiter is active, so no safety polling interval is needed.
    /// Call this once after the Store is wrapped in `Arc`.
    pub fn start_flush_task(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let store = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                store.flush_notify.notified().await;
                // Coalesce event bursts. Persisting the full tasks snapshot on
                // every parsed line is far more expensive than a periodic
                // poll and can cause thousands of fsyncs during one build.
                // This one-shot debounce only wakes after real activity.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if store.dirty.load(Ordering::Acquire) {
                    // persist_tasks acquires the mutex internally, safe to call
                    if let Err(e) = store.persist_tasks() {
                        tracing::warn!("background flush failed: {}", e);
                    }
                }
            }
        })
    }

    /// Append a single event JSON line to the per-task file.
    fn append_event_line(&self, task_id: &str, line: &str) -> Result<()> {
        let events_dir = self.dir.join("events");
        ensure_private_dir(&events_dir)?;
        let path = events_dir.join(format!("{}.jsonl", task_id));
        let mut files = self.event_files.lock().expect("event file mutex poisoned");
        if !files.contains_key(task_id) {
            files.insert(task_id.to_string(), Some(open_private_append(&path)?));
        }
        let append_result = {
            let Some(Some(file)) = files.get_mut(task_id) else {
                return Err(crate::ArshyError::Other(format!(
                    "event stream for task {task_id} was disabled after a failed partial append"
                )));
            };
            append_jsonl_line(file, line)
        };
        if let Err(failure) = append_result {
            return handle_append_failure(&mut files, task_id, failure);
        }
        Ok(())
    }

    /// Write raw output to a per-task file under `<store_dir>/raw/<task_id>.txt`.
    fn write_raw_output(&self, task_id: &str, raw_output: &str) -> Result<()> {
        let raw_dir = self.dir.join("raw");
        ensure_private_dir(&raw_dir)?;
        let path = raw_dir.join(format!("{}.txt", task_id));
        let tmp = raw_dir.join(format!("{}.txt.tmp", task_id));
        use std::io::Write as _;
        create_private_file(&tmp)?.write_all(raw_output.as_bytes())?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Read raw output from the per-task file, if it exists.
    fn read_raw_output(&self, task_id: &str) -> Result<Option<String>> {
        let path = self.dir.join("raw").join(format!("{}.txt", task_id));
        if path.exists() {
            Ok(Some(std::fs::read_to_string(&path)?))
        } else {
            Ok(None)
        }
    }

    /// Run `integrity_check` — verify tasks.jsonl is parseable.
    pub fn integrity_check(&self) -> Result<String> {
        let tasks = self.lock();
        let corrupt_task_lines = self
            .corrupt_task_lines
            .lock()
            .map_err(|_| crate::ArshyError::Other("corrupt task lines mutex poisoned".into()))?;
        if !corrupt_task_lines.is_empty() {
            let lines = corrupt_task_lines
                .iter()
                .map(|(line, _)| line.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(crate::ArshyError::Other(format!(
                "corrupt task rows in {} at line(s): {}",
                self.dir.join("tasks.jsonl").display(),
                lines
            )));
        }
        // If we loaded successfully, the data is intact
        // Verify that the events directory is accessible
        let events_dir = self.dir.join("events");
        if events_dir.exists() {
            let _event_files = self
                .event_files
                .lock()
                .map_err(|_| crate::ArshyError::Other("event file mutex poisoned".into()))?;
            // Try to read each event file
            for entry in std::fs::read_dir(&events_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    let content = std::fs::read_to_string(&path)?;
                    for (line_number, line) in content.lines().enumerate() {
                        if !line.trim().is_empty() {
                            let _: serde_json::Value = serde_json::from_str(line).map_err(|e| {
                                crate::ArshyError::Other(format!(
                                    "corrupt event in {} at line {}: {}",
                                    path.display(),
                                    line_number + 1,
                                    e
                                ))
                            })?;
                        }
                    }
                }
            }
        }
        drop(corrupt_task_lines);
        drop(tasks);
        Ok("ok".to_string())
    }

    /// Return the on-disk store directory.
    pub fn store_dir(&self) -> &Path {
        &self.dir
    }
}

type LoadedTasks = (HashMap<String, TaskRecord>, Vec<(usize, String)>);

fn load_tasks_from_disk(dir: &Path) -> Result<LoadedTasks> {
    let path = dir.join("tasks.jsonl");
    let mut map = HashMap::new();
    let mut corrupt_lines = Vec::new();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        for (lineno, raw_line) in content.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<TaskRecord>(line) {
                Ok(mut record) => {
                    // Migrate inline raw_output to file-based storage if present
                    if let Some(ref raw) = record.raw_output {
                        if !raw.is_empty() {
                            let raw_dir = dir.join("raw");
                            let _ = ensure_private_dir(&raw_dir);
                            let path = raw_dir.join(format!("{}.txt", record.task.task_id));
                            if !path.exists() {
                                if let Ok(mut file) = create_private_file(&path) {
                                    let _ = file.write_all(raw.as_bytes());
                                }
                            }
                        }
                    }
                    record.raw_output = None;
                    map.insert(record.task.task_id.clone(), record);
                }
                Err(e) => {
                    tracing::warn!("tasks.jsonl line {} corrupt, skipping: {}", lineno + 1, e);
                    corrupt_lines.push((lineno + 1, raw_line.to_string()));
                }
            }
        }
    }
    Ok((map, corrupt_lines))
}

// ── Store Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{EventLocation, QueryParams, Task, TaskEvent, TaskStatus};
    use std::io;
    use tempfile::TempDir;

    struct FaultWriter {
        bytes: Vec<u8>,
        fail_after: Option<usize>,
        fail_rollback: bool,
    }

    impl std::io::Write for FaultWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if let Some(limit) = self.fail_after {
                if self.bytes.len() >= limit {
                    return Err(io::Error::new(io::ErrorKind::WriteZero, "injected disk full"));
                }
                let count = buffer.len().min(limit - self.bytes.len());
                self.bytes.extend_from_slice(&buffer[..count]);
                return Ok(count);
            }
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl RollbackAppend for FaultWriter {
        fn current_len(&self) -> io::Result<u64> {
            Ok(self.bytes.len() as u64)
        }

        fn rollback_to(&mut self, len: u64) -> io::Result<()> {
            if self.fail_rollback {
                return Err(io::Error::other("injected rollback failure"));
            }
            self.bytes.truncate(len as usize);
            Ok(())
        }
    }

    #[test]
    fn partial_event_append_rolls_back_before_future_rows() {
        let previous = b"{\"seq\":1}\n";
        let mut file = FaultWriter {
            bytes: previous.to_vec(),
            fail_after: Some(previous.len() + 4),
            fail_rollback: false,
        };

        let failure = append_jsonl_line(&mut file, r#"{"seq":2}"#).unwrap_err();
        assert!(failure.rollback_error.is_none());
        assert_eq!(file.bytes, previous);

        file.fail_after = None;
        append_jsonl_line(&mut file, r#"{"seq":3}"#).unwrap();
        assert_eq!(file.bytes, b"{\"seq\":1}\n{\"seq\":3}\n");
        for line in file.bytes.split(|byte| *byte == b'\n').filter(|line| !line.is_empty()) {
            serde_json::from_slice::<serde_json::Value>(line).unwrap();
        }
    }

    #[test]
    fn rollback_failure_disables_the_event_stream() {
        let previous = b"{\"seq\":1}\n";
        let mut writer = FaultWriter {
            bytes: previous.to_vec(),
            fail_after: Some(previous.len() + 4),
            fail_rollback: true,
        };
        let failure = append_jsonl_line(&mut writer, r#"{"seq":2}"#).unwrap_err();
        assert!(failure.rollback_error.is_some());
        assert!(writer.bytes.len() > previous.len(), "injected write should leave a partial row");

        let (store, _tmp) = test_store();
        let task = make_task("poisoned-stream", "echo test", TaskStatus::Running);
        store.insert_task(&task).unwrap();
        let path = store.dir.join("events/poisoned-stream.jsonl");
        let file = open_private_append(&path).unwrap();
        let mut streams = store.event_files.lock().unwrap();
        streams.insert("poisoned-stream".to_string(), Some(file));
        handle_append_failure(&mut streams, "poisoned-stream", failure).unwrap_err();
        drop(streams);

        let error = store.append_event_line("poisoned-stream", r#"{"seq":3}"#).unwrap_err();
        assert!(error.to_string().contains("disabled after a failed partial append"));
        assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
    }

    /// Create a temporary store with initialized schema.
    fn test_store() -> (Store, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Store::open(&db_path).unwrap();
        store.initialize_schema().unwrap();
        (store, tmp)
    }

    fn make_task(id: &str, command: &str, status: TaskStatus) -> Task {
        Task {
            task_id: id.to_string(),
            command: command.to_string(),
            cwd: Some("/tmp".to_string()),
            status,
            exit_code: None,
            pid: None,
            parser_name: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            duration_ms: None,
            events_count: 0,
            error_count: 0,
            purpose: None,
            carrier: None,
        }
    }

    fn make_event(event_type: &str, severity: &str, message: &str) -> TaskEvent {
        TaskEvent {
            seq: 0,
            event_type: event_type.to_string(),
            severity: Some(severity.to_string()),
            code: None,
            message: message.to_string(),
            location: None,
            context: None,
            hint: None,
        }
    }

    // ── Idle watchdog tests ────────────────────────────────────────────────

    #[test]
    fn idle_since_secs_empty_store_reports_idle() {
        let (store, _tmp) = test_store();
        // An empty store (e.g. a daemon that only served short commands,
        // which never touch the store) must report an idle time — previously
        // it returned None and the daemon never idle-exited.
        assert!(store.idle_since_secs().unwrap().is_some());
    }

    #[test]
    fn idle_since_secs_running_task_returns_none() {
        let (store, _tmp) = test_store();
        let task = make_task("t1", "sleep 1", TaskStatus::Running);
        store.insert_task(&task).unwrap();
        assert!(store.idle_since_secs().unwrap().is_none());
    }

    #[test]
    fn mark_activity_refreshes_idle_time() {
        let (store, _tmp) = test_store();
        store.mark_activity();
        let secs = store.idle_since_secs().unwrap().unwrap();
        assert!(secs <= 1, "idle time should be ~0 right after activity, got {}", secs);
    }

    #[test]
    fn active_fast_command_blocks_idle_exit() {
        let (store, _tmp) = test_store();
        store.begin_fast_activity();
        assert!(store.idle_since_secs().unwrap().is_none());
        store.end_fast_activity();
        assert!(store.idle_since_secs().unwrap().is_some());
    }

    #[tokio::test]
    async fn idle_deadline_returns_without_polling_when_already_elapsed() {
        let (store, _tmp) = test_store();
        store.last_activity.store(chrono::Utc::now().timestamp() - 2, Ordering::Relaxed);
        tokio::time::timeout(std::time::Duration::from_millis(100), store.wait_until_idle(1))
            .await
            .expect("elapsed idle deadline should return immediately");
    }

    #[test]
    fn reopen_marks_interrupted_running_tasks_terminal() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("store");
        {
            let store = Store::open(&path).unwrap();
            store.insert_task(&make_task("interrupted", "sleep 60", TaskStatus::Running)).unwrap();
            store.flush().unwrap();
        }

        let reopened = Store::open(&path).unwrap();
        let task = reopened.get_task("interrupted").unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(task.finished_at.is_some());
        assert!(reopened.idle_since_secs().unwrap().is_some());
    }

    #[test]
    fn corrupt_task_rows_survive_flush_and_report_relocated_line() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("store");
        {
            let store = Store::open(&path).unwrap();
            store.insert_task(&make_task("recoverable-a", "true", TaskStatus::Completed)).unwrap();
            store.insert_task(&make_task("recoverable-b", "true", TaskStatus::Completed)).unwrap();
            store.flush().unwrap();
        }

        let tasks_path = path.join("tasks.jsonl");
        let mut original = std::fs::read(&tasks_path).unwrap();
        original.splice(0..0, b"not-json\n".iter().copied());
        std::fs::write(&tasks_path, &original).unwrap();

        let reopened = Store::open(&path).unwrap();
        let integrity_error = reopened.integrity_check().unwrap_err();
        assert!(integrity_error.to_string().contains("line(s): 1"));
        reopened.update_task("recoverable-a", &TaskStatus::Failed, Some(1), Some(0)).unwrap();
        let integrity_error = reopened.integrity_check().unwrap_err();
        assert!(integrity_error.to_string().contains("line(s): 3"));
        reopened.flush().unwrap();

        let after_flush = std::fs::read_to_string(tasks_path).unwrap();
        assert!(after_flush.lines().any(|line| line == "not-json"));
        assert!(after_flush.contains("recoverable-a"));
        assert!(after_flush.contains("recoverable-b"));
    }

    // ── Schema tests ────────────────────────────────────────────────────────

    #[test]
    fn schema_idempotent() {
        let (store, _tmp) = test_store();
        // Calling initialize_schema twice should not error
        store.initialize_schema().unwrap();
    }

    #[test]
    fn integrity_check_ok() {
        let (store, _tmp) = test_store();
        let result = store.integrity_check().unwrap();
        assert_eq!(result, "ok");
    }

    // ── Task CRUD tests ─────────────────────────────────────────────────────

    #[test]
    fn insert_and_get_task() {
        let (store, _tmp) = test_store();
        let task = make_task("task-001", "echo hello", TaskStatus::Running);
        store.insert_task(&task).unwrap();

        let fetched = store.get_task("task-001").unwrap().unwrap();
        assert_eq!(fetched.task_id, "task-001");
        assert_eq!(fetched.command, "echo hello");
        assert_eq!(fetched.status, TaskStatus::Running);
        assert_eq!(fetched.cwd, Some("/tmp".to_string()));
    }

    #[test]
    fn get_task_not_found() {
        let (store, _tmp) = test_store();
        let result = store.get_task("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn insert_task_duplicate_errors() {
        let (store, _tmp) = test_store();
        let task = make_task("dup-1", "echo a", TaskStatus::Running);
        store.insert_task(&task).unwrap();
        // Second insert with same ID should error (duplicate)
        let result = store.insert_task(&task);
        assert!(result.is_err());
    }

    #[test]
    fn list_tasks_empty() {
        let (store, _tmp) = test_store();
        let tasks = store.list_tasks(None, 10).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn list_tasks_ordered_by_started_at() {
        let (store, _tmp) = test_store();
        let mut t1 = make_task("t1", "first", TaskStatus::Running);
        t1.started_at = "2024-01-01T00:00:00Z".to_string();
        let mut t2 = make_task("t2", "second", TaskStatus::Running);
        t2.started_at = "2024-01-02T00:00:00Z".to_string();
        let mut t3 = make_task("t3", "third", TaskStatus::Running);
        t3.started_at = "2024-01-03T00:00:00Z".to_string();

        store.insert_task(&t1).unwrap();
        store.insert_task(&t2).unwrap();
        store.insert_task(&t3).unwrap();

        let tasks = store.list_tasks(None, 10).unwrap();
        assert_eq!(tasks.len(), 3);
        // DESC order: t3, t2, t1
        assert_eq!(tasks[0].task_id, "t3");
        assert_eq!(tasks[1].task_id, "t2");
        assert_eq!(tasks[2].task_id, "t1");
    }

    #[test]
    fn list_tasks_filter_by_status() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("r1", "echo a", TaskStatus::Running)).unwrap();
        store.insert_task(&make_task("c1", "echo b", TaskStatus::Completed)).unwrap();
        store.insert_task(&make_task("r2", "echo c", TaskStatus::Running)).unwrap();

        let running = store.list_tasks(Some("\"running\""), 10).unwrap();
        assert_eq!(running.len(), 2);

        let completed = store.list_tasks(Some("\"completed\""), 10).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].task_id, "c1");
    }

    #[test]
    fn list_tasks_respects_limit() {
        let (store, _tmp) = test_store();
        for i in 0..5 {
            store.insert_task(&make_task(&format!("t{}", i), "cmd", TaskStatus::Running)).unwrap();
        }
        let tasks = store.list_tasks(None, 3).unwrap();
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn update_task_status() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("u1", "echo done", TaskStatus::Running)).unwrap();

        store.update_task("u1", &TaskStatus::Completed, Some(0), Some(150)).unwrap();

        let task = store.get_task("u1").unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(task.exit_code, Some(0));
        assert_eq!(task.duration_ms, Some(150));
        assert!(task.finished_at.is_some());
    }

    #[test]
    fn update_task_pid() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("p1", "echo pid", TaskStatus::Running)).unwrap();

        store.update_task_pid("p1", 12345).unwrap();

        let task = store.get_task("p1").unwrap().unwrap();
        assert_eq!(task.pid, Some(12345));
    }

    #[test]
    fn update_nonexistent_task_no_error() {
        let (store, _tmp) = test_store();
        // UPDATE on nonexistent row silently affects 0 rows
        store.update_task("ghost", &TaskStatus::Failed, Some(1), None).unwrap();
    }

    // ── Event tests ─────────────────────────────────────────────────────────

    #[test]
    fn insert_and_query_event() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("evt-1", "cmd", TaskStatus::Running)).unwrap();

        let event = make_event("diagnostic", "error", "type mismatch at line 10");
        store.insert_event("evt-1", 1, &event).unwrap();

        let params = QueryParams {
            task_id: Some("evt-1".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].message, "type mismatch at line 10");
        assert_eq!(events[0].event_type, "diagnostic");
        assert_eq!(events[0].severity, Some("error".to_string()));
    }

    #[test]
    fn insert_event_increments_counts() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("cnt-1", "cmd", TaskStatus::Running)).unwrap();

        // Insert info event
        store.insert_event("cnt-1", 1, &make_event("log", "info", "starting")).unwrap();
        // Insert error event
        store.insert_event("cnt-1", 2, &make_event("diagnostic", "error", "fail")).unwrap();
        // Insert another info event
        store.insert_event("cnt-1", 3, &make_event("log", "info", "done")).unwrap();

        let task = store.get_task("cnt-1").unwrap().unwrap();
        assert_eq!(task.events_count, 3);
        assert_eq!(task.error_count, 1);
    }

    #[test]
    fn finalize_event_stream_preserves_corrupt_event_file() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("corrupt-evt", "cmd", TaskStatus::Running)).unwrap();
        let event = make_event("diagnostic", "error", "keep this event");
        store.insert_event("corrupt-evt", 1, &event).unwrap();

        let event_path = store.store_dir().join("events/corrupt-evt.jsonl");
        let mut original = std::fs::read(&event_path).unwrap();
        original.extend_from_slice(b"not-json\n");
        std::fs::write(&event_path, &original).unwrap();

        store.finalize_event_stream("corrupt-evt").unwrap();
        assert_eq!(std::fs::read(event_path).unwrap(), original);
    }

    #[test]
    fn finalize_event_stream_rejects_unknown_task_without_creating_orphan_file() {
        let (store, _tmp) = test_store();
        let error = store.finalize_event_stream("pruned-task").unwrap_err();

        assert!(error.to_string().contains("unknown task pruned-task"));
        assert!(!store.store_dir().join("events/pruned-task.jsonl").exists());
        assert!(!store.store_dir().join("events/pruned-task.jsonl.tmp").exists());
    }

    #[test]
    fn event_queries_report_corrupt_rows_instead_of_hiding_them() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("corrupt-query", "cmd", TaskStatus::Completed)).unwrap();
        store
            .insert_event("corrupt-query", 1, &make_event("diagnostic", "error", "visible event"))
            .unwrap();

        let event_path = store.store_dir().join("events/corrupt-query.jsonl");
        let mut original = std::fs::read(&event_path).unwrap();
        original.extend_from_slice(b"not-json\n");
        std::fs::write(&event_path, original).unwrap();

        let integrity_error = store.integrity_check().unwrap_err().to_string();
        assert!(integrity_error.contains("corrupt event"));
        assert!(integrity_error.contains("line 2"));

        let per_task = QueryParams {
            task_id: Some("corrupt-query".into()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
        };
        let error = store.query_events(&per_task).unwrap_err().to_string();
        assert!(error.contains("corrupt event"));
        assert!(error.contains("line 2"));

        let cross_task = QueryParams { task_id: None, ..per_task };
        let error = store.search_events(&cross_task).unwrap_err().to_string();
        assert!(error.contains("corrupt event"));
        assert!(error.contains("line 2"));
    }

    #[test]
    fn insert_event_with_location() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("loc-1", "cmd", TaskStatus::Running)).unwrap();

        let mut event = make_event("diagnostic", "error", "undefined var");
        event.location =
            Some(EventLocation { file: "src/main.rs".to_string(), line: 42, column: Some(10) });
        store.insert_event("loc-1", 1, &event).unwrap();

        let params = QueryParams {
            task_id: Some("loc-1".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
        };
        let (events, _) = store.query_events(&params).unwrap();
        let loc = events[0].location.as_ref().unwrap();
        assert_eq!(loc.file, "src/main.rs");
        assert_eq!(loc.line, 42);
        assert_eq!(loc.column, Some(10));
    }

    #[test]
    fn query_events_filter_by_type() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("f1", "cmd", TaskStatus::Running)).unwrap();
        store.insert_event("f1", 1, &make_event("diagnostic", "error", "err")).unwrap();
        store.insert_event("f1", 2, &make_event("summary", "info", "done")).unwrap();
        store.insert_event("f1", 3, &make_event("diagnostic", "warning", "warn")).unwrap();

        let params = QueryParams {
            task_id: Some("f1".to_string()),
            event_type: Some("diagnostic".to_string()),
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 2);
        assert!(events.iter().all(|e| e.event_type == "diagnostic"));
    }

    #[test]
    fn query_events_filter_by_severity() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("f2", "cmd", TaskStatus::Running)).unwrap();
        store.insert_event("f2", 1, &make_event("diagnostic", "error", "err")).unwrap();
        store.insert_event("f2", 2, &make_event("diagnostic", "warning", "warn")).unwrap();
        store.insert_event("f2", 3, &make_event("log", "info", "info")).unwrap();

        let params = QueryParams {
            task_id: Some("f2".to_string()),
            event_type: None,
            severity: Some("error".to_string()),
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].severity, Some("error".to_string()));
    }

    #[test]
    fn query_events_filter_by_type_and_severity() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("f3", "cmd", TaskStatus::Running)).unwrap();
        store.insert_event("f3", 1, &make_event("diagnostic", "error", "e1")).unwrap();
        store.insert_event("f3", 2, &make_event("diagnostic", "warning", "w1")).unwrap();
        store.insert_event("f3", 3, &make_event("summary", "error", "e2")).unwrap();

        let params = QueryParams {
            task_id: Some("f3".to_string()),
            event_type: Some("diagnostic".to_string()),
            severity: Some("error".to_string()),
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0].message, "e1");
    }

    #[test]
    fn query_events_pagination() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("pg1", "cmd", TaskStatus::Running)).unwrap();
        for i in 0..10 {
            store
                .insert_event("pg1", i, &make_event("log", "info", &format!("line {}", i)))
                .unwrap();
        }

        // First page
        let params = QueryParams {
            task_id: Some("pg1".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 3,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 10);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].message, "line 0");

        // Second page
        let params = QueryParams {
            task_id: Some("pg1".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 3,
            offset: 3,
            include_logs: true,
        };
        let (events, _) = store.query_events(&params).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].message, "line 3");
    }

    #[test]
    fn search_events_across_tasks() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("s1", "cmd", TaskStatus::Completed)).unwrap();
        store.insert_task(&make_task("s2", "cmd", TaskStatus::Completed)).unwrap();
        let mut e1 = make_event("diagnostic", "error", "TS2345 in alpha");
        e1.code = Some("TS2345".to_string());
        e1.location = Some(EventLocation { file: "src/a.ts".to_string(), line: 1, column: None });
        let mut e2 = make_event("diagnostic", "error", "E0308 in beta");
        e2.code = Some("E0308".to_string());
        e2.location = Some(EventLocation { file: "src/b.rs".to_string(), line: 2, column: None });
        let e3 = make_event("log", "info", "noise line");
        store.insert_event("s1", 1, &e1).unwrap();
        store.insert_event("s1", 2, &e3).unwrap();
        store.insert_event("s2", 1, &e2).unwrap();

        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: false,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 2, "log events excluded by default");
        assert_eq!(events.len(), 2);
        let ids: std::collections::HashSet<&str> =
            events.iter().map(|e| e["task_id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), 2, "events must carry their owning task_id");

        // Filter by error code across tasks.
        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: Some("TS2345".to_string()),
            file: None,
            limit: 100,
            offset: 0,
            include_logs: false,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0]["code"].as_str(), Some("TS2345"));
        assert_eq!(events[0]["task_id"].as_str(), Some("s1"));

        // Filter by file path substring across tasks.
        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: Some("src/b.rs".to_string()),
            limit: 100,
            offset: 0,
            include_logs: false,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0]["task_id"].as_str(), Some("s2"));
    }

    #[test]
    fn search_events_respects_limit_and_include_logs() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("l1", "cmd", TaskStatus::Completed)).unwrap();
        store.insert_task(&make_task("l2", "cmd", TaskStatus::Completed)).unwrap();
        for i in 0..3 {
            store.insert_event("l1", i, &make_event("log", "info", &format!("line {i}"))).unwrap();
        }
        store.insert_event("l2", 1, &make_event("diagnostic", "error", "boom")).unwrap();

        // include_logs=false → only the diagnostic across both tasks.
        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 10,
            offset: 0,
            include_logs: false,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 1);
        assert_eq!(events[0]["type"].as_str(), Some("diagnostic"));

        // include_logs=true + limit=2 → pagination over the cross-task set.
        let params = QueryParams {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 2,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.search_events(&params).unwrap();
        assert_eq!(total, 4);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn query_events_empty_result() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("empty", "cmd", TaskStatus::Running)).unwrap();

        let params = QueryParams {
            task_id: Some("empty".to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: 100,
            offset: 0,
            include_logs: true,
        };
        let (events, total) = store.query_events(&params).unwrap();
        assert_eq!(total, 0);
        assert!(events.is_empty());
    }

    // ── Prune tests ─────────────────────────────────────────────────────────

    fn insert_tasks_with_offset(store: &Store, count: usize) {
        let base = chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        for i in 0..count {
            let mut task = make_task(&format!("prune-{:03}", i), "cmd", TaskStatus::Completed);
            task.started_at = (base + chrono::Duration::days(i as i64)).to_rfc3339();
            store.insert_task(&task).unwrap();
            store
                .insert_event(&format!("prune-{:03}", i), 1, &make_event("log", "info", "msg"))
                .unwrap();
            store.finalize_event_stream(&format!("prune-{:03}", i)).unwrap();
        }
    }

    #[test]
    fn prune_keep_basic() {
        let (store, _tmp) = test_store();
        insert_tasks_with_offset(&store, 10);

        let (dt, de) = store.prune_keep(3).unwrap();
        // Keep 3 most recent → cutoff is 3rd (prune-007), delete < prune-007 = 7 tasks
        assert_eq!(dt, 7);
        assert_eq!(de, 7);

        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn prune_keep_more_than_exists() {
        let (store, _tmp) = test_store();
        insert_tasks_with_offset(&store, 3);

        let (dt, de) = store.prune_keep(10).unwrap();
        assert_eq!(dt, 0);
        assert_eq!(de, 0);

        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn prune_keep_empty() {
        let (store, _tmp) = test_store();
        let (dt, de) = store.prune_keep(5).unwrap();
        assert_eq!(dt, 0);
        assert_eq!(de, 0);
    }

    #[test]
    fn prune_keep_zero_deletes_all() {
        let (store, _tmp) = test_store();
        insert_tasks_with_offset(&store, 5);

        let (dt, de) = store.prune_keep(0).unwrap();
        assert_eq!(dt, 5);
        assert_eq!(de, 5);

        let remaining = store.list_tasks(None, 100).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn prune_older_than_deletes_old() {
        let (store, _tmp) = test_store();

        // Insert a task from 60 days ago
        let old_date = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        let mut old_task = make_task("old-1", "old cmd", TaskStatus::Completed);
        old_task.started_at = old_date;
        store.insert_task(&old_task).unwrap();
        store.insert_event("old-1", 1, &make_event("log", "info", "old")).unwrap();
        store.finalize_event_stream("old-1").unwrap();

        // Insert a task from today
        store.insert_task(&make_task("new-1", "new cmd", TaskStatus::Running)).unwrap();
        store.insert_event("new-1", 1, &make_event("log", "info", "new")).unwrap();

        let (dt, de) = store.prune_older_than(30).unwrap();
        assert_eq!(dt, 1);
        assert_eq!(de, 1);

        let remaining = store.list_tasks(None, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].task_id, "new-1");
    }

    #[test]
    fn prune_older_than_nothing_to_delete() {
        let (store, _tmp) = test_store();
        store.insert_task(&make_task("recent", "cmd", TaskStatus::Running)).unwrap();

        let (dt, de) = store.prune_older_than(30).unwrap();
        assert_eq!(dt, 0);
        assert_eq!(de, 0);
    }

    // ── Raw output tests ────────────────────────────────────────────────────

    #[test]
    fn update_and_retrieve_raw_output() {
        let (store, _tmp) = test_store();
        let task = make_task("t1", "cargo test", TaskStatus::Completed);
        store.insert_task(&task).unwrap();
        store.update_task_raw_output("t1", "line1\nline2\nline3").unwrap();
        let raw = store.get_task_raw_output("t1").unwrap();
        assert_eq!(raw.as_deref(), Some("line1\nline2\nline3"));
    }

    #[test]
    fn raw_output_none_for_missing() {
        let (store, _tmp) = test_store();
        let raw = store.get_task_raw_output("nonexistent").unwrap();
        assert!(raw.is_none());
    }

    #[test]
    fn raw_output_overwrite() {
        let (store, _tmp) = test_store();
        let task = make_task("t2", "echo test", TaskStatus::Running);
        store.insert_task(&task).unwrap();
        store.update_task_raw_output("t2", "first version").unwrap();
        store.update_task_raw_output("t2", "second version").unwrap();
        let raw = store.get_task_raw_output("t2").unwrap();
        assert_eq!(raw.as_deref(), Some("second version"));
    }

    #[cfg(unix)]
    #[test]
    fn store_data_is_owner_only_even_with_permissive_umask_and_existing_files() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("store");
        std::fs::create_dir_all(path.join("raw")).unwrap();
        std::fs::create_dir_all(path.join("events")).unwrap();
        std::fs::write(path.join("tasks.jsonl"), "").unwrap();
        std::fs::write(path.join("raw/old.txt"), "private output").unwrap();
        std::fs::write(path.join("events/old.jsonl"), "").unwrap();
        for directory in [path.clone(), path.join("raw"), path.join("events")] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        for file in
            [path.join("tasks.jsonl"), path.join("raw/old.txt"), path.join("events/old.jsonl")]
        {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        let store = Store::open(&path).unwrap();
        store.initialize_schema().unwrap();
        store.insert_task(&make_task("private", "echo secret-token", TaskStatus::Running)).unwrap();
        store.insert_event("private", 1, &make_event("log", "info", "private output")).unwrap();
        store.update_task_raw_output("private", "private output").unwrap();
        store.flush().unwrap();

        for directory in [path.clone(), path.join("raw"), path.join("events")] {
            assert_eq!(std::fs::metadata(directory).unwrap().permissions().mode() & 0o777, 0o700);
        }
        for file in [
            path.join("tasks.jsonl"),
            path.join("events/private.jsonl"),
            path.join("raw/private.txt"),
            path.join("raw/old.txt"),
            path.join("events/old.jsonl"),
        ] {
            assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn store_open_refuses_symlink_root_and_task_file() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let tmp = TempDir::new().unwrap();
        let external = tmp.path().join("external");
        std::fs::create_dir(&external).unwrap();
        std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o755)).unwrap();
        let root_link = tmp.path().join("root-link");
        symlink(&external, &root_link).unwrap();
        assert!(Store::open(&root_link).is_err());
        assert_eq!(
            std::fs::metadata(&external).unwrap().permissions().mode() & 0o777,
            0o755,
            "refusing the symlink must not chmod its target"
        );

        let store_dir = tmp.path().join("store");
        std::fs::create_dir(&store_dir).unwrap();
        let task_target = tmp.path().join("sensitive.jsonl");
        std::fs::write(&task_target, "external data").unwrap();
        let task_link = store_dir.join("tasks.jsonl");
        symlink(&task_target, &task_link).unwrap();
        assert!(Store::open(&store_dir).is_err());
        assert_eq!(std::fs::read_to_string(&task_target).unwrap(), "external data");
    }

    #[cfg(unix)]
    #[test]
    fn store_open_rejects_fifo_without_waiting_for_a_writer() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("store");
        std::fs::create_dir(&path).unwrap();
        let task_file = path.join("tasks.jsonl");
        let c_path = CString::new(task_file.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c_path` is NUL-terminated and points to a path inside our
        // temporary directory; mode is restricted to the current user.
        let result = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
        assert_eq!(result, 0, "failed to create FIFO fixture");

        let started = std::time::Instant::now();
        assert!(Store::open(&path).is_err());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "opening a corrupted store FIFO must fail promptly"
        );
    }
}
