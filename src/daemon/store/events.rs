use crate::ipc::{QueryParams, TaskEvent};
use crate::Result;

/// Update per-event metrics on a task record. Shared between insert_event
/// and merge_enriched_events to prevent logic divergence.
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
    if event.context.is_some() {
        metrics.contexts_enriched += 1;
    }
}

impl super::Store {
    /// Insert a structured event for a task.
    /// The event is written to disk immediately. In-memory counts are updated
    /// under the mutex, and the periodic background flush will persist them.
    pub fn insert_event(&self, task_id: &str, _seq: u64, event: &TaskEvent) -> Result<()> {
        // Append to per-task JSONL file
        let line = serde_json::to_string(event)?;
        self.append_event_line(task_id, &line)?;

        // Bump counts in the in-memory task record
        {
            let mut tasks = self.lock();
            if let Some(record) = tasks.get_mut(task_id) {
                record.task.events_count += 1;
                if event.severity.as_deref() == Some("error") {
                    record.task.error_count += 1;
                }
                update_event_metrics(&mut record.metrics, event, line.len() as u64);
            }
        }
        // Mark dirty so the updated counts get persisted by the background flush
        self.mark_dirty();
        Ok(())
    }

    /// Merge source-context-enriched events into the task's event file.
    ///
    /// Uses seq-based replacement: reads existing events, replaces matching seqs
    /// with enriched versions, and atomically rewrites the file.
    /// All I/O happens under the task mutex to prevent TOCTOU races with
    /// concurrent `insert_event` calls.
    pub fn merge_enriched_events(&self, task_id: &str, enriched: &[TaskEvent]) -> Result<()> {
        // Build a lookup of enriched events by seq
        let enriched_map: std::collections::HashMap<u64, &TaskEvent> =
            enriched.iter().map(|e| (e.seq, e)).collect();

        // Close the append handle before replacing the file atomically. The
        // next event append will reopen the renamed path.
        self.event_files
            .lock()
            .map_err(|_| crate::ArshyError::Other("event file mutex poisoned".into()))?
            .remove(task_id);

        // All file I/O under mutex to prevent TOCTOU race with insert_event
        let mut tasks = self.lock();

        // Read existing events from disk
        let events_dir = self.dir.join("events");
        let path = events_dir.join(format!("{}.jsonl", task_id));
        let mut all_events: Vec<TaskEvent> = Vec::new();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(event) = serde_json::from_str::<TaskEvent>(line) {
                    all_events.push(event);
                }
            }
        }

        // Merge: replace events with matching seq, keep the rest
        for event in &mut all_events {
            if let Some(enriched_evt) = enriched_map.get(&event.seq) {
                *event = (*enriched_evt).clone();
            }
        }

        // Add any enriched events that don't exist in the original
        let existing_seqs: std::collections::HashSet<u64> =
            all_events.iter().map(|e| e.seq).collect();
        for evt in enriched {
            if !existing_seqs.contains(&evt.seq) {
                all_events.push(evt.clone());
            }
        }

        // Sort by seq to maintain order
        all_events.sort_by_key(|e| e.seq);

        // Compute serialized lines and metrics in one pass
        let mut serialized_lines: Vec<String> = Vec::with_capacity(all_events.len());
        let mut local_metrics = super::TaskMetrics::default();

        for event in &all_events {
            let line = serde_json::to_string(event)?;
            let json_len = line.len() as u64;
            serialized_lines.push(line);
            update_event_metrics(&mut local_metrics, event, json_len);
        }

        let super::TaskMetrics {
            structured_events_bytes,
            visible_events,
            skipped_noise_events,
            locations_extracted,
            codes_extracted,
            contexts_enriched,
            ..
        } = local_metrics;

        // Atomically write the merged file (still under mutex)
        let tmp = events_dir.join(format!("{}.jsonl.tmp", task_id));
        let mut file = std::fs::File::create(&tmp)?;
        for line in &serialized_lines {
            std::io::Write::write_all(&mut file, line.as_bytes())?;
            std::io::Write::write_all(&mut file, b"\n")?;
        }
        file.sync_all()?;
        std::fs::rename(&tmp, &path)?;

        // Update task record metrics (already holding mutex)
        if let Some(record) = tasks.get_mut(task_id) {
            record.task.events_count = all_events.len() as u64;
            record.task.error_count =
                all_events.iter().filter(|e| e.severity.as_deref() == Some("error")).count() as u64;
            record.metrics.structured_events_bytes = structured_events_bytes;
            record.metrics.visible_events = visible_events;
            record.metrics.skipped_noise_events = skipped_noise_events;
            record.metrics.locations_extracted = locations_extracted;
            record.metrics.codes_extracted = codes_extracted;
            record.metrics.contexts_enriched = contexts_enriched;
        }
        drop(tasks);
        self.persist_tasks()
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
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(event) = serde_json::from_str::<TaskEvent>(line) else {
                continue;
            };
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
        files.sort_by(|a, b| b.0.cmp(&a.0));

        let mut total = 0usize;
        let mut events = Vec::with_capacity(params.limit.min(1024));
        for (_mtime, path) in files {
            let task_id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let content = std::fs::read_to_string(&path)?;
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(event) = serde_json::from_str::<TaskEvent>(line) {
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
        }

        Ok((events, total))
    }
}
