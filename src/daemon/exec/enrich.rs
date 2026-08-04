//! Event enrichment — source-context extraction and failure project context.

use crate::daemon::context::{git_correlator::GitCorrelation, ContextEnricher};
use crate::ipc::TaskStatus;

/// Shared enrichment function — enriches events with source context.
///
/// arshy's job is structured extraction (file:line location, error code,
/// severity, source context), **not** advice. Synthesising cause/fix hints
/// is the LLM's task — a failed command's raw output already speaks for
/// itself, and arshy only surfaces *where* the error is. The former HintDb
/// lookup has therefore been removed.
///
/// Safe to call from both sync and async contexts (uses blocking I/O internally;
/// callers should wrap with `tokio::task::spawn_blocking` when in async context).
///
/// Parameters:
/// - `events`: raw event values (serde_json::Value)
/// - `cwd`: working directory for resolving file paths in context extraction
/// - `_tool_name`: reserved (previously drove HintDb language mapping; unused now)
///
/// Returns the enriched events vector.
pub fn enrich_events(
    events: Vec<serde_json::Value>,
    cwd: &std::path::Path,
    _tool_name: Option<&str>,
) -> Vec<serde_json::Value> {
    // Context enrichment on error/warning events (file:line → surrounding source).
    let mut enricher = ContextEnricher::new(3);
    let mut task_events: Vec<crate::ipc::TaskEvent> =
        events.iter().filter_map(|e| serde_json::from_value(e.clone()).ok()).collect();
    enricher.enrich(&mut task_events, cwd);
    task_events.into_iter().map(|e| serde_json::to_value(&e).unwrap_or_default()).collect()
}

/// Filter events to only include error-severity items.
pub(crate) fn filter_events_errors_only(
    events: &Option<Vec<serde_json::Value>>,
) -> Option<Vec<serde_json::Value>> {
    events.as_ref().map(|evts| {
        evts.iter()
            .filter(|e| e.get("severity").and_then(|v| v.as_str()) == Some("error"))
            .cloned()
            .collect()
    })
}

/// Extract the first error-level event as the root cause of failure.
pub(crate) fn extract_root_cause(
    events: &Option<Vec<serde_json::Value>>,
) -> Option<serde_json::Value> {
    let evts = events.as_ref()?;
    let first =
        evts.iter().find(|e| e.get("severity").and_then(|v| v.as_str()) == Some("error"))?;
    // Traceback streams: the first error event is the uninformative
    // "Traceback (most recent call last):" banner; the real root cause is the
    // final exception line (last diagnostic error event).
    let is_traceback_banner = first
        .get("message")
        .and_then(|v| v.as_str())
        .is_some_and(|m| m.trim_start().starts_with("Traceback ("));
    if is_traceback_banner {
        return evts
            .iter()
            .rev()
            .find(|e| {
                e.get("severity").and_then(|v| v.as_str()) == Some("error")
                    && e.get("type").and_then(|v| v.as_str()) == Some("diagnostic")
            })
            .cloned()
            .or(Some(first.clone()));
    }
    Some(first.clone())
}

/// Estimate the bytes delivered to the agent for token savings telemetry.
pub(crate) fn calculate_agent_delivered_bytes(
    is_short: bool,
    raw_len: u64,
    root_cause_msg: Option<&str>,
    root_cause_file: Option<&str>,
    git_diff_stat: Option<&str>,
) -> u64 {
    if is_short {
        return raw_len;
    }
    // Base status line: "✗ 3 errors, 4 warnings   235ms (exit 1)"
    let mut size = 50;
    if let Some(msg) = root_cause_msg {
        size += msg.len() as u64 + 15;
    }
    if let Some(file) = root_cause_file {
        size += file.len() as u64 + 15;
    }
    if let Some(git) = git_diff_stat {
        size += git.len() as u64 + 15;
    }
    size
}

/// Compute project context for failed commands.
/// Runs `git diff --stat` to show recent changes, and correlates error events
/// with recently changed files via `GitCorrelation`.
pub(crate) fn compute_enhanced_project_context(
    status: &TaskStatus,
    cwd: Option<&std::path::Path>,
    events: &[serde_json::Value],
) -> Option<serde_json::Value> {
    if *status != TaskStatus::Failed {
        return None;
    }

    let mut context = serde_json::json!({});

    // Git diff stat
    let mut cmd = std::process::Command::new("git");
    cmd.args(["diff", "--stat", "HEAD~1"]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let diff_stat = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !diff_stat.is_empty() {
                context["git_diff_stat"] = serde_json::json!(diff_stat);
            }
        }
    }

    // Git correlation — match error events to recently changed files
    if let Some(gc) = GitCorrelation::detect(cwd) {
        context["changed_files"] = serde_json::json!(gc.changed_files());

        let correlated: Vec<serde_json::Value> = events
            .iter()
            .filter(|e| e.get("severity").and_then(|v| v.as_str()) == Some("error"))
            .filter_map(|e| {
                let file = e.get("location")?.get("file")?.as_str()?;
                let recently_changed = gc.changed_files().iter().any(|f| f == file);
                Some(serde_json::json!({
                    "file": file,
                    "recently_changed": recently_changed,
                }))
            })
            .collect();

        if !correlated.is_empty() {
            context["correlated_errors"] = serde_json::json!(correlated);
        }
    }

    if context.as_object().is_none_or(|m| m.is_empty()) {
        return None;
    }
    Some(context)
}
