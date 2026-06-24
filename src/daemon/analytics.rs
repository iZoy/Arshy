//! Impact analytics -- reads from the Store and computes usage metrics.
//!
//! This module is intentionally **low-coupling**: it only uses `Store`'s public
//! read-only API and reads event JSONL files from the store directory.

use serde::Serialize;
use std::collections::HashMap;

use arshy_lib::ipc::TaskEvent;

// ── Report structs ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ImpactReport {
    pub summary: SummaryMetrics,
    pub token_efficiency: TokenEfficiency,
    pub information_density: InformationDensity,
    pub command_patterns: CommandPatterns,
    pub temporal: TemporalMetrics,
    pub generated_at: String,
}

#[derive(Debug, Serialize)]
pub struct SummaryMetrics {
    pub total_tasks: u64,
    pub total_events: u64,
    pub total_errors: u64,
    pub date_range_days: u64,
    pub avg_tasks_per_day: f64,
}

#[derive(Debug, Serialize)]
pub struct TokenEfficiency {
    pub total_raw_output_bytes: u64,
    pub total_structured_bytes: u64,
    pub agent_skipped_events: u64,
    pub agent_visible_events: u64,
    pub noise_pct: f64,
    pub estimated_token_savings_pct: f64,
}

#[derive(Debug, Serialize)]
pub struct InformationDensity {
    pub avg_fields_per_event: f64,
    pub events_with_location: u64,
    pub events_with_code: u64,
    pub events_with_context: u64,
    pub events_with_hint: u64,
    pub structured_event_pct: f64,
}

#[derive(Debug, Serialize)]
pub struct CommandPatterns {
    pub unique_commands: usize,
    pub commands_run_multiple_times: usize,
    pub total_retry_runs: u64,
    pub top_retried: Vec<(String, u64)>,
    pub short_cmd_pct: f64,
    pub long_cmd_pct: f64,
}

#[derive(Debug, Serialize)]
pub struct TemporalMetrics {
    pub date_range: (String, String),
    pub span_days: u64,
    pub avg_tasks_per_day: f64,
    pub avg_events_per_day: f64,
}

// ── Analytics engine ─────────────────────────────────────────────────────────

/// Computes impact metrics from a `Store` reference without modifying it.
pub struct Analytics<'a> {
    store: &'a super::store::Store,
}

impl<'a> Analytics<'a> {
    pub fn new(store: &'a super::store::Store) -> Self {
        Self { store }
    }

    /// Generate a full `ImpactReport` from the store contents.
    ///
    /// This is read-only — the store is never modified.
    pub fn generate_report(&self) -> arshy_lib::Result<ImpactReport> {
        let tasks = self.store.list_tasks(None, 10_000)?;
        let events = self.load_all_events()?;

        let summary = self.compute_summary(&tasks, &events);
        let token_efficiency = self.compute_token_efficiency(&tasks, &events);
        let information_density = self.compute_information_density(&events);
        let command_patterns = self.compute_command_patterns(&tasks);
        let temporal = self.compute_temporal(&tasks, &summary);

        Ok(ImpactReport {
            summary,
            token_efficiency,
            information_density,
            command_patterns,
            temporal,
            generated_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    // ── Internal: data loading ──────────────────────────────────────────────

    /// Read every event JSONL file from the store's `events/` directory.
    fn load_all_events(&self) -> arshy_lib::Result<Vec<TaskEvent>> {
        let events_dir = self.store.store_dir().join("events");
        let mut all = Vec::new();

        if !events_dir.exists() {
            return Ok(all);
        }

        for entry in std::fs::read_dir(&events_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "jsonl") {
                let content = std::fs::read_to_string(&path)?;
                for line in content.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(ev) = serde_json::from_str::<TaskEvent>(line) {
                        all.push(ev);
                    }
                }
            }
        }

        Ok(all)
    }

    // ── Internal: metric computation ────────────────────────────────────────

    fn compute_summary(
        &self,
        tasks: &[arshy_lib::ipc::Task],
        events: &[TaskEvent],
    ) -> SummaryMetrics {
        let total_tasks = tasks.len() as u64;
        let total_events = events.len() as u64;
        let total_errors =
            events.iter().filter(|e| e.severity.as_deref() == Some("error")).count() as u64;

        let span = compute_span_days(tasks);
        let avg_tasks_per_day = if span > 0 { total_tasks as f64 / span as f64 } else { 0.0 };

        SummaryMetrics {
            total_tasks,
            total_events,
            total_errors,
            date_range_days: span,
            avg_tasks_per_day,
        }
    }

    fn compute_token_efficiency(
        &self,
        tasks: &[arshy_lib::ipc::Task],
        events: &[TaskEvent],
    ) -> TokenEfficiency {
        // Raw output bytes: sum of stored raw_output lengths.
        // Since we can only get them via the public API one at a time, we
        // estimate from event message bytes instead (much cheaper than N
        // store round-trips for every task).
        let total_raw_output_bytes: u64 = events.iter().map(|e| e.message.len() as u64).sum();

        // Structured bytes: JSON size of events
        let total_structured_bytes: u64 =
            events.iter().map(|e| serde_json::to_vec(e).map_or(0, |v| v.len() as u64)).sum();

        // Agent-visible events exclude "log" type (skipped by default in query_events)
        let agent_skipped_events = events.iter().filter(|e| e.event_type == "log").count() as u64;
        let agent_visible_events = events.len() as u64 - agent_skipped_events;

        let noise_pct = if !events.is_empty() {
            agent_skipped_events as f64 / events.len() as f64 * 100.0
        } else {
            0.0
        };

        // Estimated token savings: structured output is typically much smaller
        // than raw terminal output.  We approximate by comparing the number of
        // events a consumer actually reads vs total lines produced.
        let total_lines_produced: u64 = tasks.iter().map(|t| t.events_count).sum();
        let estimated_token_savings_pct = if total_lines_produced > 0 {
            let skipped = total_lines_produced.saturating_sub(agent_visible_events);
            skipped as f64 / total_lines_produced as f64 * 100.0
        } else {
            0.0
        };

        TokenEfficiency {
            total_raw_output_bytes,
            total_structured_bytes,
            agent_skipped_events,
            agent_visible_events,
            noise_pct,
            estimated_token_savings_pct,
        }
    }

    fn compute_information_density(&self, events: &[TaskEvent]) -> InformationDensity {
        let total = events.len() as u64;
        if total == 0 {
            return InformationDensity {
                avg_fields_per_event: 0.0,
                events_with_location: 0,
                events_with_code: 0,
                events_with_context: 0,
                events_with_hint: 0,
                structured_event_pct: 0.0,
            };
        }

        let mut with_location: u64 = 0;
        let mut with_code: u64 = 0;
        let mut with_context: u64 = 0;
        let mut with_hint: u64 = 0;
        let mut field_count: u64 = 0;

        for ev in events {
            // Every event always has: seq, type, message = 3 base fields
            let mut fields: u64 = 3;
            if ev.severity.is_some() {
                fields += 1;
            }
            if ev.location.is_some() {
                fields += 1;
                with_location += 1;
            }
            if ev.code.is_some() {
                fields += 1;
                with_code += 1;
            }
            if ev.context.is_some() {
                fields += 1;
                with_context += 1;
            }
            if ev.hint.is_some() {
                fields += 1;
                with_hint += 1;
            }
            field_count += fields;
        }

        // "structured" events are those with at least one enrichment field
        let structured = events
            .iter()
            .filter(|e| {
                e.location.is_some() || e.code.is_some() || e.context.is_some() || e.hint.is_some()
            })
            .count() as u64;

        InformationDensity {
            avg_fields_per_event: field_count as f64 / total as f64,
            events_with_location: with_location,
            events_with_code: with_code,
            events_with_context: with_context,
            events_with_hint: with_hint,
            structured_event_pct: structured as f64 / total as f64 * 100.0,
        }
    }

    fn compute_command_patterns(&self, tasks: &[arshy_lib::ipc::Task]) -> CommandPatterns {
        // Group by first 2 words of command
        let mut cmd_groups: HashMap<String, u64> = HashMap::new();
        let mut short_count: u64 = 0;
        let mut long_count: u64 = 0;

        for task in tasks {
            let key = command_prefix(&task.command);
            *cmd_groups.entry(key).or_insert(0) += 1;

            // Duration-based short/long classification
            match task.duration_ms {
                Some(ms) if ms < 500 => short_count += 1,
                Some(_) => long_count += 1,
                None => {
                    // Running tasks with no duration yet — classify by heuristic
                    if is_short_heuristic(&task.command) {
                        short_count += 1;
                    } else {
                        long_count += 1;
                    }
                }
            }
        }

        let total = tasks.len() as u64;
        let unique_commands = cmd_groups.len();
        let commands_run_multiple_times = cmd_groups.values().filter(|&&count| count > 1).count();
        let total_retry_runs: u64 =
            cmd_groups.values().filter(|&&count| count > 1).map(|&c| c - 1).sum();

        let mut top_retried: Vec<(String, u64)> =
            cmd_groups.into_iter().filter(|(_, count)| *count > 1).collect();
        top_retried.sort_by(|a, b| b.1.cmp(&a.1));
        top_retried.truncate(10);

        let short_cmd_pct = if total > 0 { short_count as f64 / total as f64 * 100.0 } else { 0.0 };
        let long_cmd_pct = if total > 0 { long_count as f64 / total as f64 * 100.0 } else { 0.0 };

        CommandPatterns {
            unique_commands,
            commands_run_multiple_times,
            total_retry_runs,
            top_retried,
            short_cmd_pct,
            long_cmd_pct,
        }
    }

    fn compute_temporal(
        &self,
        tasks: &[arshy_lib::ipc::Task],
        summary: &SummaryMetrics,
    ) -> TemporalMetrics {
        let dates: Vec<String> =
            tasks.iter().filter_map(|t| parse_date_part(&t.started_at)).collect();

        let date_range = if dates.is_empty() {
            (String::new(), String::new())
        } else {
            let mut sorted = dates.clone();
            sorted.sort();
            (sorted[0].clone(), sorted[sorted.len() - 1].clone())
        };

        let span_days = summary.date_range_days;
        let avg_tasks_per_day = summary.avg_tasks_per_day;
        let avg_events_per_day =
            if span_days > 0 { summary.total_events as f64 / span_days as f64 } else { 0.0 };

        TemporalMetrics { date_range, span_days, avg_tasks_per_day, avg_events_per_day }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract first 2 whitespace-delimited words from a command string.
fn command_prefix(cmd: &str) -> String {
    let words: Vec<&str> = cmd.split_whitespace().take(2).collect();
    if words.is_empty() {
        "(empty)".to_string()
    } else {
        words.join(" ")
    }
}

/// Quick heuristic: short if < 80 chars and not a known build/test prefix.
fn is_short_heuristic(cmd: &str) -> bool {
    if cmd.len() >= 80 {
        return false;
    }
    let first = cmd.split_whitespace().next().unwrap_or("");
    let long_prefixes = [
        "cargo",
        "npm",
        "yarn",
        "pnpm",
        "make",
        "cmake",
        "docker",
        "docker-compose",
        "gradle",
        "mvn",
        "sbt",
        "mix",
        "go",
        "python",
        "pytest",
        "jest",
        "mocha",
    ];
    !long_prefixes.contains(&first)
}

/// Compute span (in days) between earliest and latest task `started_at`.
fn compute_span_days(tasks: &[arshy_lib::ipc::Task]) -> u64 {
    let mut dates: Vec<chrono::NaiveDate> = tasks
        .iter()
        .filter_map(|t| {
            chrono::DateTime::parse_from_rfc3339(&t.started_at).ok().map(|dt| dt.naive_utc().date())
        })
        .collect();

    if dates.len() < 2 {
        return if dates.is_empty() { 0 } else { 1 };
    }

    dates.sort();
    let first = dates[0];
    let last = dates[dates.len() - 1];
    (last - first).num_days().max(1) as u64
}

/// Extract "YYYY-MM-DD" from an RFC 3339 timestamp string.
fn parse_date_part(ts: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(ts).ok().map(|dt| dt.naive_utc().date().to_string())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::ipc::{EventHint, EventLocation, Task, TaskEvent, TaskStatus};

    fn make_task(id: &str, command: &str, started: &str, duration: Option<u64>) -> Task {
        Task {
            task_id: id.to_string(),
            command: command.to_string(),
            cwd: Some("/tmp".to_string()),
            status: TaskStatus::Completed,
            exit_code: Some(0),
            pid: None,
            parser_name: None,
            started_at: started.to_string(),
            finished_at: None,
            duration_ms: duration,
            events_count: 0,
            error_count: 0,
        }
    }

    #[test]
    fn command_prefix_two_words() {
        assert_eq!(command_prefix("cargo test --release"), "cargo test");
        assert_eq!(command_prefix("ls"), "ls");
        assert_eq!(command_prefix(""), "(empty)");
        assert_eq!(command_prefix("  "), "(empty)");
    }

    #[test]
    fn is_short_heuristic_basic() {
        assert!(is_short_heuristic("ls -la"));
        assert!(is_short_heuristic("echo hello"));
        assert!(!is_short_heuristic("cargo build --release"));
        assert!(!is_short_heuristic("npm test"));
        // Long string
        assert!(!is_short_heuristic(&"x".repeat(80)));
    }

    #[test]
    fn compute_span_days_empty() {
        assert_eq!(compute_span_days(&[]), 0);
    }

    #[test]
    fn compute_span_days_single() {
        let tasks = vec![make_task("t1", "ls", "2024-01-01T00:00:00Z", None)];
        assert_eq!(compute_span_days(&tasks), 1);
    }

    #[test]
    fn compute_span_days_range() {
        let tasks = vec![
            make_task("t1", "ls", "2024-01-01T00:00:00Z", None),
            make_task("t2", "ls", "2024-01-11T00:00:00Z", None),
        ];
        assert_eq!(compute_span_days(&tasks), 10);
    }

    #[test]
    fn parse_date_part_valid() {
        assert_eq!(parse_date_part("2024-06-15T12:30:00Z"), Some("2024-06-15".to_string()));
    }

    #[test]
    fn parse_date_part_invalid() {
        assert_eq!(parse_date_part("not-a-date"), None);
    }

    #[test]
    fn summary_empty_data() {
        let store = test_store();
        let analytics = Analytics::new(&store);
        let summary = analytics.compute_summary(&[], &[]);
        assert_eq!(summary.total_tasks, 0);
        assert_eq!(summary.total_events, 0);
        assert_eq!(summary.total_errors, 0);
        assert_eq!(summary.date_range_days, 0);
        assert_eq!(summary.avg_tasks_per_day, 0.0);
    }

    #[test]
    fn information_density_empty() {
        let store = test_store();
        let analytics = Analytics::new(&store);
        let density = analytics.compute_information_density(&[]);
        assert_eq!(density.avg_fields_per_event, 0.0);
        assert_eq!(density.events_with_location, 0);
    }

    #[test]
    fn information_density_counts_fields() {
        let store = test_store();
        let analytics = Analytics::new(&store);

        let events = vec![
            TaskEvent {
                seq: 0,
                event_type: "diagnostic".into(),
                severity: Some("error".into()),
                code: Some("E0001".into()),
                message: "something failed".into(),
                location: Some(EventLocation { file: "main.rs".into(), line: 42, column: None }),
                context: None,
                hint: Some(EventHint {
                    cause: "bad input".into(),
                    fix: Some("check args".into()),
                    retry: None,
                }),
            },
            TaskEvent {
                seq: 1,
                event_type: "log".into(),
                severity: None,
                code: None,
                message: "raw line".into(),
                location: None,
                context: None,
                hint: None,
            },
        ];

        let density = analytics.compute_information_density(&events);
        assert_eq!(density.events_with_location, 1);
        assert_eq!(density.events_with_code, 1);
        assert_eq!(density.events_with_hint, 1);
        assert_eq!(density.events_with_context, 0);
        // 50% of events have at least one enrichment field
        assert!((density.structured_event_pct - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn command_patterns_grouping() {
        let store = test_store();
        let analytics = Analytics::new(&store);

        let tasks = vec![
            make_task("t1", "cargo test", "2024-01-01T00:00:00Z", Some(1000)),
            make_task("t2", "cargo test --release", "2024-01-01T01:00:00Z", Some(2000)),
            make_task("t3", "cargo build", "2024-01-01T02:00:00Z", Some(5000)),
            make_task("t4", "ls -la", "2024-01-01T03:00:00Z", Some(50)),
        ];

        let patterns = analytics.compute_command_patterns(&tasks);
        // "cargo test" and "cargo build" are two unique prefixes
        assert_eq!(patterns.unique_commands, 3); // "cargo test", "cargo build", "ls -la"
                                                 // "cargo test" ran twice => 1 group run multiple times
        assert_eq!(patterns.commands_run_multiple_times, 1);
        assert_eq!(patterns.total_retry_runs, 1); // 2 runs - 1 = 1 retry
        assert_eq!(patterns.top_retried[0].0, "cargo test");
        assert_eq!(patterns.top_retried[0].1, 2);
    }

    #[test]
    fn token_efficiency_noise_pct() {
        let store = test_store();
        let analytics = Analytics::new(&store);

        let events = vec![
            TaskEvent {
                seq: 0,
                event_type: "diagnostic".into(),
                severity: Some("error".into()),
                code: None,
                message: "error".into(),
                location: None,
                context: None,
                hint: None,
            },
            TaskEvent {
                seq: 1,
                event_type: "log".into(),
                severity: None,
                code: None,
                message: "raw".into(),
                location: None,
                context: None,
                hint: None,
            },
            TaskEvent {
                seq: 2,
                event_type: "log".into(),
                severity: None,
                code: None,
                message: "raw2".into(),
                location: None,
                context: None,
                hint: None,
            },
        ];

        let efficiency = analytics.compute_token_efficiency(&[], &events);
        assert_eq!(efficiency.agent_skipped_events, 2);
        assert_eq!(efficiency.agent_visible_events, 1);
        // 2/3 = 66.67%
        assert!((efficiency.noise_pct - 66.66666666666666).abs() < 0.01);
    }

    // ── Test helper ─────────────────────────────────────────────────────────

    fn test_store() -> super::super::store::Store {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = super::super::store::Store::open(&db_path, false).unwrap();
        store.initialize_schema().unwrap();
        store
    }
}
