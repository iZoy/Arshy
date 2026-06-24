use arshy_lib::Result;

impl super::Store {
    /// Ensure storage directories exist. Replaces the old CREATE TABLE statements.
    pub fn initialize_schema(&self) -> Result<()> {
        let events_dir = self.dir.join("events");
        std::fs::create_dir_all(&events_dir)?;
        Ok(())
    }

    /// No-op — WAL mode is SQLite-specific.
    pub fn wal_checkpoint(&self) -> Result<()> {
        Ok(())
    }

    /// Return aggregate stats about tasks and events.
    pub fn get_stats(
        &self,
        db_path: Option<&std::path::Path>,
    ) -> Result<arshy_lib::ipc::StatsResponse> {
        use arshy_lib::ipc::StatusCounts;

        let tasks = self.lock().clone();
        let total_tasks = tasks.len() as u64;

        let mut counts = StatusCounts::default();
        let mut durations: Vec<u64> = Vec::new();
        let mut total_events: u64 = 0;
        let mut total_errors: u64 = 0;
        let mut dedup_total: u64 = 0;
        let mut correlated_total: u64 = 0;
        let mut total_raw_output_bytes: u64 = 0;
        let mut total_structured_events_bytes: u64 = 0;
        let mut total_agent_visible_events: u64 = 0;
        let mut total_agent_skipped_events: u64 = 0;
        let mut total_locations_extracted: u64 = 0;
        let mut total_codes_extracted: u64 = 0;
        let mut total_contexts_enriched: u64 = 0;
        let mut total_hints_attached: u64 = 0;
        let mut parser_usage: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        for record in tasks.values() {
            match &record.task.status {
                arshy_lib::ipc::TaskStatus::Running => counts.running += 1,
                arshy_lib::ipc::TaskStatus::Completed => counts.completed += 1,
                arshy_lib::ipc::TaskStatus::Failed => counts.failed += 1,
                arshy_lib::ipc::TaskStatus::Killed => counts.killed += 1,
                arshy_lib::ipc::TaskStatus::Timeout => counts.timeout += 1,
            }
            if let Some(d) = record.task.duration_ms {
                durations.push(d);
            }
            total_events += record.task.events_count;
            total_errors += record.task.error_count;
            dedup_total += record.dedup_collapsed;
            correlated_total += record.correlated_errors;
            total_raw_output_bytes += record.metrics.raw_output_bytes;
            total_structured_events_bytes += record.metrics.structured_events_bytes;
            total_agent_visible_events += record.metrics.agent_visible_events;
            total_agent_skipped_events += record.metrics.agent_skipped_events;
            total_locations_extracted += record.metrics.locations_extracted;
            total_codes_extracted += record.metrics.codes_extracted;
            total_contexts_enriched += record.metrics.contexts_enriched;
            total_hints_attached += record.metrics.hints_attached;
            if let Some(ref parser) = record.task.parser_name {
                *parser_usage.entry(parser.clone()).or_insert(0) += 1;
            }
        }

        // Use pre-computed TaskMetrics for parser coverage, hints, and context.
        // Falls back to scanning event files only when metrics are zero (legacy data).
        let needs_scan = total_agent_visible_events == 0 && total_agent_skipped_events == 0;
        let mut parser_coverage_non_log = total_agent_visible_events;
        let mut hints_count = total_hints_attached;
        let mut context_count = total_contexts_enriched;

        let events_dir = self.dir.join("events");
        if needs_scan && events_dir.exists() {
            parser_coverage_non_log = 0;
            hints_count = 0;
            context_count = 0;
            for entry in std::fs::read_dir(&events_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                let content = std::fs::read_to_string(&path)?;
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<arshy_lib::ipc::TaskEvent>(line) {
                        if event.event_type != "log" {
                            parser_coverage_non_log += 1;
                        }
                        if event.hint.is_some() {
                            hints_count += 1;
                        }
                        if event.context.is_some() {
                            context_count += 1;
                        }
                    }
                }
            }
        }

        let parser_coverage_pct = if total_events == 0 {
            None
        } else {
            Some(parser_coverage_non_log as f64 * 100.0 / total_events as f64)
        };

        durations.sort_unstable();
        let avg_duration = if durations.is_empty() {
            None
        } else {
            Some(durations.iter().sum::<u64>() as f64 / durations.len() as f64)
        };
        let p50 = percentile(&durations, 50);
        let p99 = percentile(&durations, 99);

        let failure_rate = if total_tasks > 0 {
            let failed_total = counts.failed + counts.timeout;
            Some(failed_total as f64 / total_tasks as f64)
        } else {
            None
        };

        // db_size: sum of all store files
        let db_size = if let Some(p) = db_path {
            // Legacy: use the provided path size if it exists
            std::fs::metadata(p).ok().map(|m| m.len())
        } else {
            // Compute total store size
            let mut total: u64 = 0;
            let tasks_file = self.dir.join("tasks.jsonl");
            if let Ok(m) = std::fs::metadata(&tasks_file) {
                total += m.len();
            }
            let versions_file = self.dir.join("versions.json");
            if let Ok(m) = std::fs::metadata(&versions_file) {
                total += m.len();
            }
            let events_dir = self.dir.join("events");
            if events_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&events_dir) {
                    for entry in entries.flatten() {
                        if let Ok(m) = entry.metadata() {
                            total += m.len();
                        }
                    }
                }
            }
            Some(total)
        };

        let hints_attached = if hints_count > 0 { Some(hints_count) } else { None };
        let context_enriched = if context_count > 0 { Some(context_count) } else { None };

        let per_parser_usage = if parser_usage.is_empty() {
            None
        } else {
            let mut list: Vec<arshy_lib::ipc::ParserCount> = parser_usage
                .into_iter()
                .map(|(parser, count)| arshy_lib::ipc::ParserCount { parser, count })
                .collect();
            list.sort_by(|a, b| b.count.cmp(&a.count));
            list.truncate(10);
            Some(list)
        };

        Ok(arshy_lib::ipc::StatsResponse {
            total_tasks,
            by_status: counts,
            total_events,
            total_errors,
            avg_duration_ms: avg_duration,
            p50_duration_ms: p50,
            p99_duration_ms: p99,
            failure_rate,
            db_size_bytes: db_size,
            parser_coverage_pct,
            hints_attached,
            context_enriched,
            dedup_collapsed: Some(dedup_total),
            correlated_errors: Some(correlated_total),
            per_parser_usage,
            total_raw_output_bytes: if total_raw_output_bytes > 0 {
                Some(total_raw_output_bytes)
            } else {
                None
            },
            total_structured_events_bytes: if total_structured_events_bytes > 0 {
                Some(total_structured_events_bytes)
            } else {
                None
            },
            total_agent_visible_events: if total_agent_visible_events > 0 {
                Some(total_agent_visible_events)
            } else {
                None
            },
            total_agent_skipped_events: if total_agent_skipped_events > 0 {
                Some(total_agent_skipped_events)
            } else {
                None
            },
            total_locations_extracted: if total_locations_extracted > 0 {
                Some(total_locations_extracted)
            } else {
                None
            },
            total_codes_extracted: if total_codes_extracted > 0 {
                Some(total_codes_extracted)
            } else {
                None
            },
            total_contexts_enriched: if total_contexts_enriched > 0 {
                Some(total_contexts_enriched)
            } else {
                None
            },
            total_hints_attached: if total_hints_attached > 0 {
                Some(total_hints_attached)
            } else {
                None
            },
        })
    }
}

fn percentile(sorted: &[u64], pct: u64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = (pct as usize * (sorted.len() - 1)) / 100;
    sorted.get(idx).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::ipc::{Task, TaskEvent, TaskStatus};
    use tempfile::TempDir;

    fn test_store() -> (super::super::Store, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("test.db");
        let store = super::super::Store::open(&db, false).unwrap();
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

    #[test]
    fn percentile_empty() {
        assert_eq!(percentile(&[], 50), None);
    }

    #[test]
    fn percentile_single() {
        assert_eq!(percentile(&[42], 50), Some(42));
        assert_eq!(percentile(&[42], 99), Some(42));
    }

    #[test]
    fn percentile_multiple() {
        let data = [10, 20, 30, 40, 50];
        assert_eq!(percentile(&data, 0), Some(10));
        assert_eq!(percentile(&data, 50), Some(30));
        assert_eq!(percentile(&data, 100), Some(50));
    }

    #[test]
    fn wal_checkpoint_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("test.db");
        let store = super::super::Store::open(&db, false).unwrap();
        store.initialize_schema().unwrap();
        // Should not error even without WAL mode
        store.wal_checkpoint().unwrap();
    }

    #[test]
    fn stats_empty_store() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("test.db");
        let store = super::super::Store::open(&db, false).unwrap();
        store.initialize_schema().unwrap();

        let stats = store.get_stats(Some(&db)).unwrap();
        assert_eq!(stats.total_tasks, 0);
        assert_eq!(stats.total_events, 0);
        assert!(stats.avg_duration_ms.is_none());
    }

    #[test]
    fn stats_intelligence_metrics() {
        let (store, _tmp) = test_store();
        let task = make_task("t1", "cargo test", TaskStatus::Failed);
        store.insert_task(&task).unwrap();
        // Insert an event with hint in payload
        let hint_event = TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: Some("E0308".into()),
            message: "mismatched types".into(),
            location: None,
            context: None,
            hint: Some(arshy_lib::ipc::EventHint {
                cause: "Type mismatch".into(),
                fix: Some("Use .into()".into()),
                retry: None,
            }),
        };
        store.insert_event("t1", 0, &hint_event).unwrap();
        store.insert_event("t1", 1, &make_event("log", "info", "line2")).unwrap();
        store.insert_event("t1", 2, &make_event("log", "info", "line3")).unwrap();

        let stats = store.get_stats(None).unwrap();
        assert!(stats.parser_coverage_pct.is_some());
        // 1 of 3 events is type != 'log'
        let coverage = stats.parser_coverage_pct.unwrap();
        assert!(coverage > 30.0 && coverage < 40.0); // ~33.3%
                                                     // 1 event has hint
        assert_eq!(stats.hints_attached, Some(1));
    }

    #[test]
    fn stats_feature_usage_counters() {
        let (store, _tmp) = test_store();
        let mut task = make_task("t1", "cargo build", TaskStatus::Completed);
        task.parser_name = Some("cargo".into());
        store.insert_task(&task).unwrap();
        let mut task2 = make_task("t2", "tsc", TaskStatus::Completed);
        task2.parser_name = Some("tsc".into());
        store.insert_task(&task2).unwrap();

        store.update_task_counters("t1", 5, 2).unwrap();
        store.update_task_counters("t2", 0, 0).unwrap();

        let stats = store.get_stats(None).unwrap();
        assert_eq!(stats.dedup_collapsed, Some(5));
        assert_eq!(stats.correlated_errors, Some(2));

        let parsers = stats.per_parser_usage.unwrap();
        assert_eq!(parsers.len(), 2);
        // Both have count 1, order may vary
        let names: Vec<&str> = parsers.iter().map(|p| p.parser.as_str()).collect();
        assert!(names.contains(&"cargo"));
        assert!(names.contains(&"tsc"));
    }

    #[test]
    fn update_task_counters_noop_for_missing() {
        let (store, _tmp) = test_store();
        // Should not error for nonexistent task
        store.update_task_counters("nonexistent", 10, 5).unwrap();
    }

    #[test]
    fn schema_migration_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = tmp.path().join("test.db");
        let store = super::super::Store::open(&db, false).unwrap();
        store.initialize_schema().unwrap();
        // Second call should not error
        store.initialize_schema().unwrap();
    }
}
