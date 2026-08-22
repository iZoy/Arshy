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

    // Git diff stat — ONLY when cwd is set AND points to a real directory.
    // When cwd is None we'd otherwise fall back to the daemon's own cwd
    // (typically the arshy source repo), polluting agent_delivered_bytes
    // with diff stat that has nothing to do with the user's command. Bail
    // early instead. (Phase B Q-3 fix, see commit history.)
    //
    // Also cap the diff stat at GIT_DIFF_STAT_MAX_BYTES — even legitimate
    // cwd calls can come from huge repos where `git diff --stat HEAD~1` is
    // many MB. The first 2 KB gives the agent enough signal about which
    // files changed recently.
    const GIT_DIFF_STAT_MAX_BYTES: usize = 2048;
    let diff_cwd = cwd.filter(|dir| dir.is_dir());

    if let Some(dir) = diff_cwd {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["diff", "--stat", "HEAD~1"]);
        cmd.current_dir(dir);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let mut diff_stat = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if diff_stat.len() > GIT_DIFF_STAT_MAX_BYTES {
                    // Truncate at the last newline to avoid mid-line cuts.
                    let cut = &diff_stat[..GIT_DIFF_STAT_MAX_BYTES];
                    let pos = cut.rfind('\n').unwrap_or(GIT_DIFF_STAT_MAX_BYTES);
                    diff_stat = format!("{}\n... [truncated]", &diff_stat[..pos]);
                }
                if !diff_stat.is_empty() {
                    context["git_diff_stat"] = serde_json::json!(diff_stat);
                }
            }
        }
    }

    // Git correlation — same defensive cwd handling. detect() with None
    // falls back to the daemon's cwd; we deliberately pass diff_cwd (which
    // is None when cwd is invalid) so correlation skips too.
    if let Some(gc) = GitCorrelation::detect(diff_cwd) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// When cwd is None, we MUST skip git_diff_stat — otherwise the daemon
    /// leaks its own repo's diff into agent_delivered_bytes. This was the
    /// root cause of the -931% savings regression. (Phase B Q-3 fix.)
    #[test]
    fn compute_project_context_skips_git_diff_when_cwd_is_none() {
        let result = compute_enhanced_project_context(&TaskStatus::Failed, None, &[]);
        // Either None (no git info available) or Some with NO git_diff_stat.
        if let Some(ctx) = result {
            assert!(
                ctx.get("git_diff_stat").is_none(),
                "cwd=None must not produce git_diff_stat — found: {:?}",
                ctx.get("git_diff_stat")
            );
        }
    }

    /// When cwd points to a non-existent directory, also skip git_diff.
    /// (Defensive against deleted temp dirs etc.)
    #[test]
    fn compute_project_context_skips_git_diff_for_nonexistent_cwd() {
        let result = compute_enhanced_project_context(
            &TaskStatus::Failed,
            Some(Path::new("/tmp/this-path-does-not-exist-arshy-test-9999")),
            &[],
        );
        if let Some(ctx) = result {
            assert!(
                ctx.get("git_diff_stat").is_none(),
                "nonexistent cwd must not produce git_diff_stat"
            );
        }
    }

    /// When cwd is set and IS a git repo, we should still produce diff stat
    /// (this is the normal path). Skips if git is not installed.
    ///
    /// Setup: init repo, commit empty (HEAD~1 doesn't exist yet), commit again
    /// with a file added — HEAD~1 now has an empty tree, HEAD has the file.
    /// \`git diff --stat HEAD~1\` shows the file as added.
    #[test]
    fn compute_project_context_produces_git_diff_for_real_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path();
        let cfg = ["-c", "user.email=t@x", "-c", "user.name=t"];

        let run = |args: &[&str]| -> bool {
            std::process::Command::new("git")
                .args(args)
                .current_dir(path)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };

        if !run(&["init", "--quiet", "--initial-branch=main"]) {
            eprintln!("skip: git init failed");
            return;
        }
        // First empty commit (so HEAD~1 is valid)
        let mut base_args: Vec<&str> = cfg.to_vec();
        base_args.extend_from_slice(&["commit", "--allow-empty", "-m", "base", "--quiet"]);
        if !run(&base_args) {
            eprintln!("skip: initial commit failed");
            return;
        }
        // Add a file and commit it (HEAD differs from HEAD~1)
        std::fs::write(path.join("marker.txt"), b"hello\n").unwrap();
        if !run(&["add", "."]) {
            eprintln!("skip: git add failed");
            return;
        }
        let mut marker_args: Vec<&str> = cfg.to_vec();
        marker_args.extend_from_slice(&["commit", "-m", "add", "--quiet"]);
        if !run(&marker_args) {
            eprintln!("skip: marker commit failed");
            return;
        }

        // Sanity: git diff --stat HEAD~1 should now succeed and mention marker.txt.
        let verify = std::process::Command::new("git")
            .args(["diff", "--stat", "HEAD~1"])
            .current_dir(path)
            .output()
            .expect("git diff must run");
        assert!(
            verify.status.success(),
            "test setup: git diff --stat HEAD~1 should succeed, stderr={}",
            String::from_utf8_lossy(&verify.stderr)
        );
        assert!(
            String::from_utf8_lossy(&verify.stdout).contains("marker.txt"),
            "test setup: git diff --stat HEAD~1 should mention marker.txt, got stdout={}",
            String::from_utf8_lossy(&verify.stdout)
        );

        let result = compute_enhanced_project_context(&TaskStatus::Failed, Some(path), &[]);
        let ctx = result.expect("should produce context for valid git repo with diff");
        let gds = ctx
            .get("git_diff_stat")
            .expect("should have git_diff_stat")
            .as_str()
            .expect("git_diff_stat must be a string");
        assert!(
            gds.contains("marker.txt"),
            "git_diff_stat should mention the marker file, got: {:?}",
            gds
        );
    }

    /// Huge git diff output is capped at 2KB to prevent agent_delivered_bytes
    /// from blowing up on big repos.
    #[test]
    fn compute_project_context_caps_git_diff_output() {
        // Create a git repo with a HUGE diff to trigger the cap.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path();
        let init =
            std::process::Command::new("git").args(["init", "--quiet"]).current_dir(path).output();
        if init.is_err() || !init.as_ref().unwrap().status.success() {
            eprintln!("skip: git not installed");
            return;
        }
        // Initial commit on empty tree
        let _ = std::process::Command::new("git")
            .args([
                "-c",
                "user.email=t@x",
                "-c",
                "user.name=t",
                "commit",
                "--allow-empty",
                "-m",
                "base",
                "--quiet",
            ])
            .current_dir(path)
            .output();

        // Create one file with > 2KB of content so HEAD~1 diff is large
        let big = "x".repeat(5000);
        std::fs::write(path.join("big.txt"), big.as_bytes()).unwrap();
        let _ = std::process::Command::new("git").args(["add", "."]).current_dir(path).output();

        let result = compute_enhanced_project_context(&TaskStatus::Failed, Some(path), &[]);
        if let Some(ctx) = result {
            if let Some(gds) = ctx.get("git_diff_stat").and_then(|v| v.as_str()) {
                // Cap is 2048 bytes but we append "\n... [truncated]" — so
                // max reasonable length is ~2080 bytes.
                assert!(
                    gds.len() < 2100,
                    "git_diff_stat should be capped near 2048 bytes, got {}",
                    gds.len()
                );
                // Truncation marker should be present.
                assert!(
                    gds.contains("[truncated]"),
                    "capped output should contain truncation marker, got first 200: {:?}",
                    &gds[..200.min(gds.len())]
                );
            }
        }
    }
}
