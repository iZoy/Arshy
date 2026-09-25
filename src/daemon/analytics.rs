//! Impact analytics -- reads from the Store and computes usage metrics.
//!
//! This module is intentionally **low-coupling**: it only uses `Store`'s public
//! read-only API and reads event JSONL files from the store directory.

use serde::Serialize;
#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;

use crate::ipc::TaskEvent;

// ── Report structs ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ImpactReport {
    pub summary: SummaryMetrics,
    pub efficiency: crate::ipc::EfficiencyReport,
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
pub struct InformationDensity {
    pub avg_fields_per_event: f64,
    pub events_with_location: u64,
    pub events_with_code: u64,
    pub structured_event_pct: f64,
}

#[derive(Debug, Serialize)]
pub struct CommandPatterns {
    pub unique_commands: usize,
    pub carrier_distribution: CarrierDistribution,
}

/// Q1 telemetry (ADR-0006): what actually executed — direct CLI invocation
/// vs bash composition vs a Python interpreter vs another script interpreter.
/// Typed-tool calls never reach arshy and are not observable here.
#[derive(Debug, Default, Serialize)]
pub struct CarrierDistribution {
    pub shell: u64,
    pub shell_composite: u64,
    pub python: u64,
    pub script_other: u64,
    pub unknown: u64,
}

// Historical-only test helper. Repair-loop inference is not production code or
// part of the public analytics contract.
#[cfg(test)]
struct RepairLoopMetrics {
    fix_loops: u64,
    avg_retries_to_fix: f64,
    avg_fix_duration_ms: u64,
    fastest_fix_ms: u64,
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
    pub fn generate_report(&self) -> crate::Result<ImpactReport> {
        let tasks = self.store.list_tasks(None, 10_000)?;
        let events = self.load_all_events()?;
        let stats = self.store.get_stats()?;

        let summary = self.compute_summary(&tasks, &events);

        let information_density = self.compute_information_density(&events);
        let command_patterns = self.compute_command_patterns(&tasks);
        let temporal = self.compute_temporal(&tasks, &summary);

        Ok(ImpactReport {
            summary,
            efficiency: stats.efficiency.unwrap_or(crate::ipc::EfficiencyReport {
                schema_version: "quality-v2".into(),
                components: crate::ipc::EfficiencyComponents {
                    content_convergence_pct: None,
                    noise_filter_pct: None,
                    diagnostic_completeness_pct: None,
                    dedup_reduction_pct: None,
                },
                counters: crate::ipc::EfficiencyCounters {
                    raw_output_bytes: 0,
                    structured_output_bytes: 0,
                    visible_events: 0,
                    skipped_noise_events: 0,
                    dedup_collapsed_events: 0,
                    locations_extracted: 0,
                    codes_extracted: 0,
                },
            }),
            information_density,
            command_patterns,
            temporal,
            generated_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    // ── Internal: data loading ──────────────────────────────────────────────

    /// Read every event JSONL file from the store's `events/` directory.
    fn load_all_events(&self) -> crate::Result<Vec<TaskEvent>> {
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

    fn compute_summary(&self, tasks: &[crate::ipc::Task], events: &[TaskEvent]) -> SummaryMetrics {
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

    fn compute_information_density(&self, events: &[TaskEvent]) -> InformationDensity {
        let total = events.len() as u64;
        if total == 0 {
            return InformationDensity {
                avg_fields_per_event: 0.0,
                events_with_location: 0,
                events_with_code: 0,
                structured_event_pct: 0.0,
            };
        }

        let mut with_location: u64 = 0;
        let mut with_code: u64 = 0;
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
            field_count += fields;
        }

        // "structured" events are those with at least one enrichment field
        let structured =
            events.iter().filter(|e| e.location.is_some() || e.code.is_some()).count() as u64;

        InformationDensity {
            avg_fields_per_event: field_count as f64 / total as f64,
            events_with_location: with_location,
            events_with_code: with_code,
            structured_event_pct: structured as f64 / total as f64 * 100.0,
        }
    }

    fn compute_command_patterns(&self, tasks: &[crate::ipc::Task]) -> CommandPatterns {
        // Only report directly observed facts. Retry counts and short/long
        // percentages depend on heuristics and execution mode, so they are
        // intentionally not product metrics.
        let mut commands = HashSet::new();
        let mut carriers = CarrierDistribution::default();

        for task in tasks {
            commands.insert(task.command.clone());
            match task.carrier.as_deref().unwrap_or("unknown") {
                "shell" => carriers.shell += 1,
                "shell_composite" => carriers.shell_composite += 1,
                "python" => carriers.python += 1,
                "script_other" => carriers.script_other += 1,
                _ => carriers.unknown += 1,
            }
        }

        CommandPatterns { unique_commands: commands.len(), carrier_distribution: carriers }
    }

    /// Conservative fix-loop detection.
    ///
    /// Within the same `cwd`, a **failure streak** — one or more error tasks
    /// (error-severity events) followed by a success task (completed, exit 0)
    /// — counts as one repair loop when the streak closes within ≤5 attempts
    /// and a ≤30-minute window. Oversized streaks (more retries or longer than
    /// the bounds) never count, and their errors cannot start new loops, which
    /// prevents long failure streaks from being miscounted as fast fixes.
    /// Only unlabeled tasks (real development workload) participate —
    /// purpose-tagged loads (dogfood/sample) contain deliberate failures and
    /// would pollute the metric. Scanning resumes after the closing success.
    #[cfg(test)]
    fn compute_repair_loop(&self, tasks: &[crate::ipc::Task]) -> RepairLoopMetrics {
        const MAX_RETRIES: usize = 5;
        const MAX_WINDOW: chrono::Duration = chrono::Duration::minutes(30);

        let mut by_cwd: HashMap<&str, Vec<&crate::ipc::Task>> = HashMap::new();
        for task in tasks {
            // Real-development workload only (no purpose label).
            if task.purpose.is_none() {
                if let Some(cwd) = task.cwd.as_deref() {
                    by_cwd.entry(cwd).or_default().push(task);
                }
            }
        }

        let mut fix_loops: u64 = 0;
        let mut retries_sum: u64 = 0;
        let mut duration_sum_ms: u64 = 0;
        let mut fastest_fix_ms: Option<u64> = None;

        let is_error = |t: &crate::ipc::Task| t.error_count > 0;
        let is_success = |t: &crate::ipc::Task| {
            t.status == crate::ipc::TaskStatus::Completed && t.exit_code == Some(0)
        };

        for group in by_cwd.values_mut() {
            // RFC 3339 timestamps sort lexicographically within a UTC timeline.
            group.sort_by(|a, b| a.started_at.cmp(&b.started_at));
            let times: Vec<Option<chrono::DateTime<chrono::FixedOffset>>> = group
                .iter()
                .map(|t| chrono::DateTime::parse_from_rfc3339(&t.started_at).ok())
                .collect();

            let mut i = 0;
            while i < group.len() {
                let (Some(t0), true) = (times[i], is_error(group[i])) else {
                    i += 1;
                    continue;
                };

                // Scan the streak: errors keep it open; a success may close it.
                let mut closed_at: Option<usize> = None;
                let mut oversized = false;
                for j in (i + 1)..group.len() {
                    if j - i > MAX_RETRIES {
                        oversized = true;
                        break;
                    }
                    let Some(tj) = times[j] else {
                        oversized = true;
                        break;
                    };
                    if tj.signed_duration_since(t0) > MAX_WINDOW {
                        oversized = true;
                        break;
                    }
                    if is_success(group[j]) {
                        closed_at = Some(j);
                        break;
                    }
                }

                match closed_at {
                    Some(j) => {
                        fix_loops += 1;
                        let retries = (j - i) as u64;
                        retries_sum += retries;
                        let duration_ms =
                            times[j].unwrap().signed_duration_since(t0).num_milliseconds().max(0)
                                as u64;
                        duration_sum_ms += duration_ms;
                        fastest_fix_ms =
                            Some(fastest_fix_ms.map_or(duration_ms, |f: u64| f.min(duration_ms)));
                        i = j + 1; // resume after the closing success
                    }
                    None if oversized => {
                        // The streak exceeded the bounds: skip to the next
                        // success (or the end) without counting anything —
                        // errors inside an oversized streak must not open
                        // fresh loops against the same late success.
                        let mut k = i + 1;
                        while k < group.len() && !is_success(group[k]) {
                            k += 1;
                        }
                        i = k.saturating_add(1);
                    }
                    None => {
                        i += 1; // unclosed streak: next task may open a new one
                    }
                }
            }
        }

        RepairLoopMetrics {
            fix_loops,
            avg_retries_to_fix: if fix_loops > 0 {
                retries_sum as f64 / fix_loops as f64
            } else {
                0.0
            },
            avg_fix_duration_ms: if fix_loops > 0 {
                (duration_sum_ms as f64 / fix_loops as f64).round() as u64
            } else {
                0
            },
            fastest_fix_ms: fastest_fix_ms.unwrap_or(0),
        }
    }

    fn compute_temporal(
        &self,
        tasks: &[crate::ipc::Task],
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

/// Compute span (in days) between earliest and latest task `started_at`.
fn compute_span_days(tasks: &[crate::ipc::Task]) -> u64 {
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
    use crate::ipc::{EventLocation, Task, TaskEvent, TaskStatus};

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
            purpose: None,
            carrier: None,
        }
    }

    fn as_error(mut t: Task) -> Task {
        t.status = TaskStatus::Failed;
        t.exit_code = Some(1);
        t.error_count = 1;
        t
    }

    #[test]
    fn repair_loop_basic_detection() {
        let tasks = vec![
            as_error(make_task("t1", "cargo build", "2026-08-03T10:00:00Z", None)),
            make_task("t2", "cargo build", "2026-08-03T10:00:20Z", None),
        ];
        let store = test_store();
        let metrics = Analytics::new(&store).compute_repair_loop(&tasks);
        assert_eq!(metrics.fix_loops, 1);
        assert_eq!(metrics.avg_retries_to_fix, 1.0);
        assert_eq!(metrics.avg_fix_duration_ms, 20_000);
        assert_eq!(metrics.fastest_fix_ms, 20_000);
    }

    #[test]
    fn repair_loop_requires_success_after_error() {
        let tasks = vec![as_error(make_task("t1", "cargo build", "2026-08-03T10:00:00Z", None))];
        let store = test_store();
        let metrics = Analytics::new(&store).compute_repair_loop(&tasks);
        assert_eq!(metrics.fix_loops, 0);
        assert_eq!(metrics.avg_retries_to_fix, 0.0);
        assert_eq!(metrics.fastest_fix_ms, 0);
    }

    #[test]
    fn repair_loop_ignores_purpose_tagged_workloads() {
        let mut error = as_error(make_task("t1", "rustc x.rs", "2026-08-03T10:00:00Z", None));
        error.purpose = Some("dogfood".to_string());
        let mut success = make_task("t2", "rustc x.rs", "2026-08-03T10:00:10Z", None);
        success.purpose = Some("dogfood".to_string());
        let store = test_store();
        let metrics = Analytics::new(&store).compute_repair_loop(&[error, success]);
        assert_eq!(metrics.fix_loops, 0, "deliberate dogfood failures must not count");
    }

    #[test]
    fn repair_loop_requires_same_cwd() {
        let mut error = as_error(make_task("t1", "cargo build", "2026-08-03T10:00:00Z", None));
        error.cwd = Some("/proj-a".to_string());
        let mut success = make_task("t2", "cargo build", "2026-08-03T10:00:10Z", None);
        success.cwd = Some("/proj-b".to_string());
        let store = test_store();
        let metrics = Analytics::new(&store).compute_repair_loop(&[error, success]);
        assert_eq!(metrics.fix_loops, 0);
    }

    #[test]
    fn repair_loop_bounded_by_retry_count_and_window() {
        let store = test_store();
        let make = |i: usize, secs: u64| {
            make_task(
                &format!("t{i}"),
                "cargo build",
                &format!("2026-08-03T10:00:{secs:02}Z"),
                None,
            )
        };
        let failed = |mut t: Task| {
            t.status = TaskStatus::Failed;
            t.exit_code = Some(1);
            t.error_count = 1;
            t
        };

        // 6 retries between the first error and the success → exceeds the bound.
        let too_many = vec![
            as_error(make(0, 0)),
            failed(make(1, 1)),
            failed(make(2, 2)),
            failed(make(3, 3)),
            failed(make(4, 4)),
            failed(make(5, 5)),
            make(6, 6),
        ];
        let metrics = Analytics::new(&store).compute_repair_loop(&too_many);
        assert_eq!(metrics.fix_loops, 0, "6 retries exceed the 5-attempt bound");

        // 3 retries between the first error and the success → counts.
        let within = vec![as_error(make(0, 0)), failed(make(1, 1)), failed(make(2, 2)), make(3, 3)];
        let metrics = Analytics::new(&store).compute_repair_loop(&within);
        assert_eq!(metrics.fix_loops, 1);
        assert_eq!(metrics.avg_retries_to_fix, 3.0);

        // Error at 10:00, success at 10:40 → outside the 30-minute window.
        let late =
            vec![as_error(make(0, 0)), make_task("s", "cargo build", "2026-08-03T10:40:00Z", None)];
        let metrics = Analytics::new(&store).compute_repair_loop(&late);
        assert_eq!(metrics.fix_loops, 0);
    }

    #[test]
    fn repair_loop_multiple_loops_and_averages() {
        let tasks = vec![
            as_error(make_task("a1", "cargo build", "2026-08-03T10:00:00Z", None)),
            make_task("a2", "cargo build", "2026-08-03T10:00:10Z", None),
            as_error(make_task("b1", "cargo test", "2026-08-03T11:00:00Z", None)),
            as_error(make_task("b2", "cargo test", "2026-08-03T11:00:20Z", None)),
            make_task("b3", "cargo test", "2026-08-03T11:00:25Z", None),
        ];
        let store = test_store();
        let metrics = Analytics::new(&store).compute_repair_loop(&tasks);
        assert_eq!(metrics.fix_loops, 2);
        // Loop A: 1 retry / 10s; loop B: 2 retries / 25s.
        assert_eq!(metrics.avg_retries_to_fix, 1.5);
        assert_eq!(metrics.avg_fix_duration_ms, 17_500);
        assert_eq!(metrics.fastest_fix_ms, 10_000);
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
                hint: None,
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
        assert_eq!(patterns.unique_commands, 4);
    }

    // ── Test helper ─────────────────────────────────────────────────────────

    fn test_store() -> super::super::store::Store {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = super::super::store::Store::open(&db_path).unwrap();
        store.initialize_schema().unwrap();
        store
    }
}
