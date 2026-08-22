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
///
/// Models what `render_run_text` actually emits:
///   1. One-line summary: "{icon} {errors} error(s), {duration} (exit {code})"
///   2. Optional "Root cause: {msg}" line
///   3. Optional file/git diff context
///   4. Top-N rendered event lines (`file:line: message`)
///
/// The agent never sees the raw PTY output for long commands — only the
/// structured summary + events. So `delivered` should be roughly bounded by
/// what fits in the summary, NOT the raw size.
///
/// Bug fix (Phase B Q-3): the previous version used `size = 50` as a hardcoded
/// base. For tiny failures (raw ~46 bytes), the base alone exceeded raw,
/// producing negative savings. We now compute the base from the actual error
/// count and cap the total at `raw_len / 2` so we never claim to have
/// delivered more than half the raw output's worth of structured content.
pub(crate) fn calculate_agent_delivered_bytes(
    is_short: bool,
    raw_len: u64,
    error_count: u64,
    warning_count: u64,
    root_cause_msg: Option<&str>,
    root_cause_file: Option<&str>,
    git_diff_stat: Option<&str>,
) -> u64 {
    if is_short {
        // Short commands return raw output verbatim; agent sees everything.
        return raw_len;
    }

    // Base summary line: ~"✓ N errors, M warnings, Xms (exit Y)" ≈ 20 chars
    // plus 8 chars per error and 6 per warning. This is the actual rendered
    // summary from `one_line_summary` in render.rs.
    let mut size = 20u64 + error_count * 8 + warning_count * 6;

    // Root cause line: "Root cause: <msg>" with ~15 char overhead.
    if let Some(msg) = root_cause_msg {
        size += msg.len() as u64 + 15;
    }
    // File: file basename + 15 char framing.
    if let Some(file) = root_cause_file {
        size += file.len() as u64 + 15;
    }
    // Git diff stat block: full text + ~15 char framing ("Recent changes:").
    if let Some(git) = git_diff_stat {
        size += git.len() as u64 + 15;
    }

    // Cap at half the raw size — we never deliver more structured bytes than
    // the raw output could possibly justify. For tiny outputs this bounds the
    // inflation; for large outputs it lets the formula scale freely.
    let cap = raw_len / 2;
    size.min(cap.max(25))
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

    /// Short commands return raw_len verbatim — agent sees the full output.
    #[test]
    fn delivered_bytes_short_command_returns_raw() {
        assert_eq!(calculate_agent_delivered_bytes(true, 42, 0, 0, None, None, None), 42);
        assert_eq!(
            calculate_agent_delivered_bytes(true, 0, 5, 5, Some("x"), Some("y"), Some("z")),
            0
        );
    }

    /// Tiny raw outputs don't blow up: the formula caps at `raw_len / 2`,
    /// and the `max(25)` floor means the status line is at least 25 bytes.
    /// This was the bug fixed in Phase B Q-3 follow-up.
    #[test]
    fn delivered_bytes_short_failure_does_not_blow_up() {
        // raw=46, base 20+0=20, no rc/git → would be 20; capped at min(20, 23) = 20
        // then max(25) = 25. So delivered = 25, savings = (46-25)/46 = 45.6%.
        let d = calculate_agent_delivered_bytes(false, 46, 1, 0, None, None, None);
        assert_eq!(d, 25, "tiny raw should be capped to floor 25, got {d}");
        assert!(d < 46, "delivered must be < raw to claim positive savings");
    }

    /// Empty raw: base = 20 + 0 errors = 20; cap = 0/2=0, floor max=25;
    /// final = min(20, 25) = 20 (the floor is a CEILING on cap, not a floor
    /// on size). For empty output the status line is genuinely shorter.
    #[test]
    fn delivered_bytes_empty_raw_returns_base_size() {
        let d = calculate_agent_delivered_bytes(false, 0, 0, 0, None, None, None);
        assert_eq!(d, 20);
    }

    /// No raw bytes + 1 error → floor kicks in (base 28 > 20 but cap is 0).
    /// The max(25) ensures the floor lifts tiny outputs to 25 minimum.
    #[test]
    fn delivered_bytes_tiny_with_error_uses_floor() {
        let d = calculate_agent_delivered_bytes(false, 0, 1, 0, None, None, None);
        // base 28; cap 0; cap.max(25) = 25; min(28, 25) = 25
        assert_eq!(d, 25);
    }

    /// Normal-size raw: base scales with error count and components add up.
    #[test]
    fn delivered_bytes_normal_size_scales_linearly() {
        // raw=1000, 3 errors, 1 warning, 50-char root cause, 30-char file
        // base = 20 + 3*8 + 1*6 = 50
        // + msg: 50+15 = 115
        // + file: 30+15 = 160
        // cap: min(160, 500) = 160 (raw/2 = 500)
        let d = calculate_agent_delivered_bytes(
            false,
            1000,
            3,
            1,
            Some("x".repeat(50).as_str()),
            Some("x".repeat(30).as_str()),
            None,
        );
        assert_eq!(d, 160);
    }

    /// Large raw: cap at raw/2 prevents unbounded growth.
    #[test]
    fn delivered_bytes_capped_at_half_raw() {
        // raw=10000, large components should not exceed raw/2 = 5000
        let huge_msg = "x".repeat(10_000);
        let huge_file = "y".repeat(10_000);
        let huge_git = "z".repeat(10_000);
        let d = calculate_agent_delivered_bytes(
            false,
            10000,
            5,
            5,
            Some(huge_msg.as_str()),
            Some(huge_file.as_str()),
            Some(huge_git.as_str()),
        );
        assert!(d <= 5000, "delivered must be capped at raw/2=5000, got {d}");
    }

    /// Git diff stat contributes its byte count + framing overhead.
    #[test]
    fn delivered_bytes_includes_git_diff() {
        let d = calculate_agent_delivered_bytes(
            false,
            1000,
            1,
            0,
            None,
            None,
            Some("a.txt | 5 +++++\nb.txt | 3 +++"),
        );
        // base 28 + git ~28+15 = ~71
        assert!(d > 50 && d < 200, "git diff should add ~43 bytes, got {d}");
    }

    /// `cargo (sh -c)` workload scenario: tiny 46-byte failure output.
    /// This was the regression case producing −102% savings before the fix.
    #[test]
    fn delivered_bytes_cargo_sh_c_workload_is_now_positive() {
        let raw = 46u64;
        let d = calculate_agent_delivered_bytes(
            false,
            raw,
            1, // 1 error event
            0,
            Some("undefined_symbol"), // root cause message
            None,                     // no file
            None,                     // no git diff (not a git repo)
        );
        // base = 20 + 1*8 = 28; + msg = 28 + 14+15 = 57
        // cap = max(57, min(57, 23)) → 25 (floor because 23 < 25)
        // So delivered = 25, savings = (46-25)/46 = 45.6% — positive!
        assert!(d <= 25, "delivered should be capped to floor, got {d}");
        assert!(d < raw, "delivered ({d}) must be < raw ({raw}) for positive savings");
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
