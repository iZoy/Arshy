use crate::ipc::{QueryParams, TaskEvent};
use crate::Result;

/// Update per-event metrics on a task record.
fn update_event_metrics(metrics: &mut super::TaskMetrics, event: &TaskEvent, json_bytes: u64) {
    metrics.structured_events_bytes += json_bytes;
    if event.event_type == "log" {
        metrics.skipped_noise_events += 1;
    } else {
        metrics.visible_events += 1;
    }
    if event.location.is_some() {
        metrics.locations_extracted += 1;
    }
    if event.code.is_some() {
        metrics.codes_extracted += 1;
    }
}

impl super::Store {
    /// Insert a structured event for a task.
    /// The event is written to disk immediately. In-memory counts are updated
    /// under the task mutex, and the periodic background flush will persist
    /// them. Taking the task mutex before the event-file mutex serializes this
    /// append with stream finalization and prune cleanup.
    pub fn insert_event(&self, task_id: &str, _seq: u64, event: &TaskEvent) -> Result<()> {
        let mut tasks = self.lock();
        if !tasks.contains_key(task_id) {
            return Err(crate::ArshyError::Other(format!(
                "cannot append event for unknown task {task_id}"
            )));
        }
        let line = serde_json::to_string(event)?;
        self.append_event_line(task_id, &line)?;

        // Bump counts in the in-memory task record
        if let Some(record) = tasks.get_mut(task_id) {
            record.task.events_count += 1;
            if event.severity.as_deref() == Some("error") {
                record.task.error_count += 1;
            }
            update_event_metrics(&mut record.metrics, event, line.len() as u64);
        }
        drop(tasks);
        // Mark dirty so the updated counts get persisted by the background flush
        self.mark_dirty();
        Ok(())
    }

    /// Close and sync a completed task's event stream before it can be pruned.
    pub fn finalize_event_stream(&self, task_id: &str) -> Result<()> {
        let tasks = self.lock();
        if !tasks.contains_key(task_id) {
            return Err(crate::ArshyError::Other(format!(
                "cannot finalize events for unknown task {task_id}"
            )));
        }
        let file = self
            .event_files
            .lock()
            .map_err(|_| crate::ArshyError::Other("event file mutex poisoned".into()))?
            .remove(task_id);
        drop(tasks);
        if let Some(Some(file)) = file {
            file.sync_all()?;
        }
        Ok(())
    }

    /// Whether an event matches the query filters. Shared by the per-task
    /// and cross-task query paths so the two can never drift apart.
    fn event_matches(event: &TaskEvent, params: &QueryParams) -> bool {
        // Skip raw log events by default — they're unstructured lines
        // that add bulk without helping agents.
        if !params.include_logs && event.event_type == "log" {
            return false;
        }
        if let Some(t) = &params.event_type {
            if event.event_type != *t {
                return false;
            }
        }
        if let Some(s) = &params.severity {
            if event.severity.as_ref() != Some(s) {
                return false;
            }
        }
        if let Some(c) = &params.code {
            if event.code.as_ref() != Some(c) {
                return false;
            }
        }
        if let Some(f) = &params.file {
            match &event.location {
                Some(loc) if loc.file.contains(f.as_str()) => {}
                _ => return false,
            }
        }
        true
    }

    /// Query a single task's events with optional filters.
    /// Returns (events, total_count).
    pub fn query_events(&self, params: &QueryParams) -> Result<(Vec<TaskEvent>, usize)> {
        let (events, summary) = self.query_events_with_summary(params)?;
        Ok((events, summary.total as usize))
    }

    /// Query events and return counts for the complete filtered set. This
    /// prevents response metadata from being capped by the pagination limit.
    pub fn query_events_with_summary(
        &self,
        params: &QueryParams,
    ) -> Result<(Vec<TaskEvent>, super::EventSummary)> {
        // Serialize reads with append writes so an in-flight JSON line is not
        // reported as store corruption merely because it was read mid-write.
        let _event_files = self
            .event_files
            .lock()
            .map_err(|_| crate::ArshyError::Other("event file mutex poisoned".into()))?;
        let task_id = params.task_id.as_deref().ok_or_else(|| {
            crate::ArshyError::Ipc(
                "task_id required for per-task query; omit it to search across all tasks".into(),
            )
        })?;
        let path = self.dir.join("events").join(format!("{}.jsonl", task_id));

        if !path.exists() {
            return Ok((vec![], super::EventSummary::default()));
        }

        let content = std::fs::read_to_string(&path)?;
        let mut events = Vec::with_capacity(params.limit.min(1024));
        let mut summary = super::EventSummary::default();
        for (line_number, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let event = serde_json::from_str::<TaskEvent>(line).map_err(|error| {
                crate::ArshyError::Other(format!(
                    "corrupt event in {} at line {}: {}",
                    path.display(),
                    line_number + 1,
                    error
                ))
            })?;
            if Self::event_matches(&event, params) {
                let index = summary.total as usize;
                summary.total += 1;
                match event.severity.as_deref() {
                    Some("error") => summary.errors += 1,
                    Some("warning") => summary.warnings += 1,
                    _ => {}
                }
                if index >= params.offset && events.len() < params.limit {
                    events.push(event);
                }
            }
        }

        Ok((events, summary))
    }

    /// Search events across **all** tasks (execution-memory query).
    ///
    /// `params.task_id` must be `None`. Returns JSON event objects with the
    /// owning `task_id` injected, newest task first (events-file mtime) with
    /// stable per-task ordering, so offset/limit pagination is deterministic.
    pub fn search_events(&self, params: &QueryParams) -> Result<(Vec<serde_json::Value>, usize)> {
        // Hold the append lock for a consistent cross-task scan.
        let _event_files = self
            .event_files
            .lock()
            .map_err(|_| crate::ArshyError::Other("event file mutex poisoned".into()))?;
        let events_dir = self.dir.join("events");
        if !events_dir.exists() {
            return Ok((vec![], 0));
        }

        // Newest task first: sort event files by mtime so pagination is stable.
        let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&events_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let mtime = entry.metadata()?.modified().unwrap_or(std::time::UNIX_EPOCH);
            files.push((mtime, path));
        }
        files.sort_by_key(|file| std::cmp::Reverse(file.0));

        let mut total = 0usize;
        let mut events = Vec::with_capacity(params.limit.min(1024));
        for (_mtime, path) in files {
            let task_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let content = std::fs::read_to_string(&path)?;
            for (line_number, line) in content.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let event = serde_json::from_str::<TaskEvent>(line).map_err(|error| {
                    crate::ArshyError::Other(format!(
                        "corrupt event in {} at line {}: {}",
                        path.display(),
                        line_number + 1,
                        error
                    ))
                })?;
                if Self::event_matches(&event, params) {
                    if total >= params.offset && events.len() < params.limit {
                        let mut value = serde_json::to_value(event).unwrap_or_default();
                        if let serde_json::Value::Object(map) = &mut value {
                            map.insert(
                                "task_id".to_string(),
                                serde_json::Value::String(task_id.clone()),
                            );
                        }
                        events.push(value);
                    }
                    total += 1;
                }
            }
        }

        Ok((events, total))
    }
}
