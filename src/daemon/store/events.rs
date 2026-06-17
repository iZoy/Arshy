use arshy_lib::ipc::{QueryParams, TaskEvent};
use arshy_lib::Result;

impl super::Store {
    /// Insert a structured event for a task.
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
            }
        }
        self.persist_tasks()
    }

    /// Query events with optional filters. Returns (events, total_count).
    pub fn query_events(&self, params: &QueryParams) -> Result<(Vec<TaskEvent>, usize)> {
        let path = self.dir.join("events").join(format!("{}.jsonl", params.task_id));

        if !path.exists() {
            return Ok((vec![], 0));
        }

        let content = std::fs::read_to_string(&path)?;
        let mut all_events: Vec<TaskEvent> = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<TaskEvent>(line) {
                Ok(event) => all_events.push(event),
                Err(_) => continue,
            }
        }

        // Apply filters
        let filtered: Vec<&TaskEvent> = all_events
            .iter()
            .filter(|e| {
                if let Some(t) = &params.event_type {
                    if e.event_type != *t {
                        return false;
                    }
                }
                if let Some(s) = &params.severity {
                    if e.severity.as_ref() != Some(s) {
                        return false;
                    }
                }
                if let Some(c) = &params.code {
                    if e.code.as_ref() != Some(c) {
                        return false;
                    }
                }
                if let Some(f) = &params.file {
                    match &e.location {
                        Some(loc) if loc.file.contains(f.as_str()) => {}
                        _ => return false,
                    }
                }
                true
            })
            .collect();

        let total = filtered.len();

        // Apply pagination (offset + limit)
        let events: Vec<TaskEvent> =
            filtered.into_iter().skip(params.offset).take(params.limit).cloned().collect();

        Ok((events, total))
    }
}
