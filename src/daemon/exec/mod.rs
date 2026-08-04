//! Execution engine — spawns commands, manages task lifecycle, streams events.
//!
//! Execution modes:
//! - **sync**: Wait for completion, return full result (blocking).
//! - **async**: Return immediately with task_id, stream events via notifications.
//! - **auto**: Smart — short commands get zero-overhead sync path,
//!   long commands get async + structured output.

pub mod process;
pub mod pty;

mod background;
mod cwd;
mod decision;
mod enrich;

#[cfg(test)]
mod tests;

pub(crate) use background::{run_background, BackgroundTask, CompletionInfo};
pub(crate) use cwd::prepare_cwd;
pub(crate) use decision::is_short_command;
pub(crate) use enrich::{
    calculate_agent_delivered_bytes, compute_enhanced_project_context, enrich_events,
    extract_root_cause, filter_events_errors_only,
};

use crate::ipc::{Task, TaskStatus};
use crate::ArshyError;
use crate::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex as TokioMutex};

use super::bus::EventBus;
use super::ipc_handler::RunResult;
use super::parser::Engine;
use super::security::{AuditEntry, AuditLog, CommandFilter, RateLimiter};
use super::store::Store;

/// Core executor that owns the store, parser, and event bus.
pub struct Executor {
    store: Arc<Store>,
    parser: Arc<Engine>,
    event_bus: EventBus,
    config: ExecutorConfig,
    filter: CommandFilter,
    sandbox_paths: Vec<String>,
    access_level: String,
    audit_log: Option<Arc<AuditLog>>,
    /// Registry of running tasks' kill signal senders.
    kill_registry: Arc<TokioMutex<HashMap<String, tokio::sync::mpsc::Sender<()>>>>,
    rate_limiter: Arc<TokioMutex<RateLimiter>>,
}

/// Runtime configuration for task execution.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub max_task_duration_ms: u64,
    pub max_output_bytes: u64,
    pub kill_graceful_ms: u64,
    pub kill_force_ms: u64,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_task_duration_ms: 3_600_000, // 1 hour
            max_output_bytes: 10_485_760,    // 10 MB
            kill_graceful_ms: 3_000,
            kill_force_ms: 2_000,
        }
    }
}

impl Executor {
    pub fn new(store: Arc<Store>, parser: Arc<Engine>, event_bus: EventBus) -> Self {
        Self {
            store,
            parser,
            event_bus,
            config: ExecutorConfig::default(),
            filter: CommandFilter::permissive(),
            sandbox_paths: Vec::new(),
            access_level: "full".into(),
            audit_log: None,
            kill_registry: Arc::new(TokioMutex::new(HashMap::new())),
            rate_limiter: Arc::new(TokioMutex::new(RateLimiter::disabled())),
        }
    }

    pub fn with_config(mut self, config: ExecutorConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_security(mut self, config: &crate::config::SecurityConfig) -> Self {
        self.filter = CommandFilter::from_config(config).unwrap_or_else(|e| {
            tracing::error!("security config: {} — using permissive fallback", e);
            CommandFilter::permissive()
        });
        self.sandbox_paths = config.sandbox_paths.clone();
        self.access_level = config.access_level.clone();

        // Initialize rate limiter from config
        let rate_limiter = if config.rate_limit.enabled {
            RateLimiter::new(config.rate_limit.burst, config.rate_limit.max_commands_per_second)
        } else {
            RateLimiter::disabled()
        };
        self.rate_limiter = Arc::new(TokioMutex::new(rate_limiter));

        self
    }

    pub fn with_audit_log(mut self, audit_log: Arc<AuditLog>) -> Self {
        self.audit_log = Some(audit_log);
        self
    }

    /// Current access level ("full" or "read-only").
    pub fn access_level(&self) -> &str {
        &self.access_level
    }

    /// Reload parsers from disk and return a human-readable diff.
    pub fn reload_parsers(&self) -> Result<String> {
        self.parser.reload()
    }

    /// Kill all running tasks. Used during daemon shutdown to drain work.
    pub async fn kill_all(&self) {
        let registry = self.kill_registry.lock().await;
        let count = registry.len();
        if count > 0 {
            tracing::info!("killing {} running task(s)", count);
        }
        for (_task_id, tx) in registry.iter() {
            let _ = tx.send(()).await;
        }
    }

    /// Schedule a command for execution.
    ///
    /// Mode behavior:
    /// - **sync**: Wait for completion, full structured path.
    /// - **async**: Return immediately with task_id, events stream via notifications.
    /// - **auto**: Smart — short commands get zero-overhead sync path (raw stdout),
    ///   long commands get sync with 60s timeout (structured result in 1 call).
    ///   Only commands exceeding 60s degrade to async (returns task_id).
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        &self,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        mode: &str,
        parse_hint: Option<&str>,
        env: Option<&HashMap<String, String>>,
        errors_only: bool,
        purpose: Option<&str>,
    ) -> Result<RunResult> {
        // ── Rate limit check (always run) ───────────────────────────────────
        {
            let mut limiter = self.rate_limiter.lock().await;
            if !limiter.try_acquire() {
                tracing::warn!("rate limit exceeded for command: {}", command);
                return Err(ArshyError::Ipc(
                    "rate limit exceeded: too many commands per second".into(),
                ));
            }
        }

        // ── Security checks (always run) ──────────────────────────────────
        if let Err(e) = self.filter.check(command) {
            if let Some(ref audit) = self.audit_log {
                let _ = audit.log(&AuditEntry {
                    timestamp: chrono::Utc::now(),
                    task_id: String::new(),
                    command: command.to_string(),
                    cwd: cwd.map(String::from),
                    exit_code: None,
                    blocked: true,
                    reason: Some(e.to_string()),
                });
            }
            return Err(e);
        }

        let check_result = {
            let cwd_owned = cwd.unwrap_or(".").to_string();
            let paths = self.sandbox_paths.clone();
            tokio::task::spawn_blocking(move || super::security::check_path(&cwd_owned, &paths))
                .await
                .map_err(|e| ArshyError::Ipc(format!("sandbox check panicked: {}", e)))?
        };
        check_result?;

        // ── CWD accessibility check & symlink fallback ──────────────────
        // macOS TCC restricts PTY child processes from accessing ~/Documents etc.
        // If the target cwd is restricted, create a symlink at /tmp/.arshy-cwd/<hash>
        // that bypasses TCC, and inject ARSHY_CWD so the command knows the real path.
        let cwd_path = cwd.map(std::path::PathBuf::from);
        let mut effective_env = env.cloned();
        let (fallback_cwd, symlink_to_cleanup) = if let Some(ref real) = cwd_path {
            prepare_cwd(real, &mut effective_env)
        } else {
            (None, None)
        };
        let spawn_cwd = fallback_cwd.or_else(|| cwd_path.clone());
        let env_for_spawn = effective_env.as_ref().or(env);

        let is_auto = mode == "auto";
        let is_explicit_sync = mode == "sync";
        let is_short = is_short_command(command);
        // When the agent provides a parse_hint, it expects structured output —
        // bypass the zero-overhead short path to ensure parser processing.
        let has_hint = parse_hint.is_some();

        // ── Auto + short (no hint) → zero-overhead fast path ──────────────
        if is_auto && is_short && !has_hint {
            super::telemetry::record_task_created();
            let spawn_cwd_str = spawn_cwd.as_ref().map(|p| p.to_string_lossy().to_string());
            let result =
                self.run_short(command, spawn_cwd_str.as_deref(), timeout_ms, env_for_spawn).await;
            // Clean up symlink fallback if used
            if let Some(ref link) = symlink_to_cleanup {
                let _ = std::fs::remove_file(link);
            }
            let result = result?;
            super::telemetry::record_task_completed(result.status == TaskStatus::Completed);
            return Ok(result);
        }

        // ── Full structured path ──────────────────────────────────────────
        // When parse_hint names a parser, use it directly; otherwise auto-detect.
        let tool = if let Some(hint) = parse_hint {
            self.parser.get_by_name(hint).or_else(|| self.parser.detect(command))
        } else {
            self.parser.detect(command)
        };
        let task_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let cwd_string = cwd.map(String::from);

        let task = Task {
            task_id: task_id.clone(),
            command: command.to_string(),
            cwd: cwd_string.clone(),
            status: TaskStatus::Running,
            exit_code: None,
            pid: None,
            parser_name: tool.as_ref().map(|t| t.parser_name.clone()),
            started_at: now,
            finished_at: None,
            duration_ms: None,
            events_count: 0,
            error_count: 0,
            purpose: purpose.map(String::from),
        };
        self.store.insert_task(&task)?;
        // Store detected tool name for async enrichment (Bug #2 fix)
        if let Some(ref t) = tool {
            let _ = self.store.set_detected_tool(&task_id, &t.tool_name);
        }

        // Auto + non-short → sync (wait for completion, with 30s timeout)
        // Explicit sync/async → as-is
        let is_sync = is_explicit_sync || (is_auto && !is_short);

        // Auto mode uses a bounded wait; if command exceeds 60s, degrade to async.
        // Explicit sync waits indefinitely (caller chose to block).
        let auto_sync_timeout =
            if is_auto && !is_short { Some(std::time::Duration::from_secs(60)) } else { None };

        // Spawn background execution task
        let store = self.store.clone();
        let store_for_enrichment = store.clone();
        let parser = self.parser.clone();
        let event_bus = self.event_bus.clone();
        let executor_config = self.config.clone();
        let cmd = command.to_string();
        let task_id_bg = task_id.clone();
        let detected_tool = tool;
        let detected_tool_clone = detected_tool.clone();

        super::telemetry::record_task_created();

        let (done_tx, done_rx) = if is_sync {
            let (tx, rx) = oneshot::channel::<CompletionInfo>();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let (kill_tx, kill_rx) = tokio::sync::mpsc::channel::<()>(1);
        self.kill_registry.lock().await.insert(task_id.clone(), kill_tx);

        let kill_registry = self.kill_registry.clone();
        let task = BackgroundTask {
            task_id: task_id_bg,
            command: cmd,
            cwd: spawn_cwd,
            timeout_ms,
            detected_tool,
            store,
            parser,
            event_bus,
            config: executor_config,
            audit_log: self.audit_log.clone(),
            done_tx,
            kill_rx,
            env: effective_env.or_else(|| env.cloned()),
        };
        let task_id_cleanup = task_id.clone();
        let symlink_cleanup = symlink_to_cleanup;
        tokio::spawn(async move {
            if let Err(e) = run_background(task).await {
                tracing::error!("background task failed: {}", e);
            }
            // Clean up symlink fallback if used
            if let Some(ref link) = symlink_cleanup {
                let _ = std::fs::remove_file(link);
            }
            kill_registry.lock().await.remove(&task_id_cleanup);
        });

        // Async mode: return immediately (explicit async, or auto timeout fallback)
        if !is_sync {
            return Ok(RunResult {
                task_id,
                status: TaskStatus::Running,
                exit_code: None,
                duration_ms: None,
                error_count: None,
                warning_count: None,
                raw_output: None,
                short_command: false,
                root_cause: None,
                project_context: None,
            });
        }

        // Sync mode: wait for completion (with optional auto timeout)
        match done_rx {
            Some(rx) => {
                let completion = if let Some(timeout) = auto_sync_timeout {
                    match tokio::time::timeout(timeout, rx).await {
                        Ok(Ok(info)) => Ok(info),
                        Ok(Err(_)) => Err(()),
                        Err(_) => {
                            // Auto mode timeout — degrade to async: task is still running,
                            // agent can query later or subscribe if needed.
                            tracing::debug!(
                                "auto sync timeout for task {}, degrading to async",
                                task_id
                            );
                            return Ok(RunResult {
                                task_id,
                                status: TaskStatus::Running,
                                exit_code: None,
                                duration_ms: None,
                                error_count: None,
                                warning_count: None,
                                raw_output: None,
                                short_command: false,
                                root_cause: None,
                                project_context: None,
                            });
                        }
                    }
                } else {
                    rx.await.map_err(|_| ())
                };

                match completion {
                    Ok(info) => {
                        // Query events from store — only used for summary/root_cause computation.
                        // The full events array is NOT sent to the agent; use arshy_query for detail.
                        let events_json: Option<Vec<serde_json::Value>> = {
                            let params = crate::ipc::QueryParams {
                                task_id: Some(task_id.clone()),
                                event_type: None,
                                severity: None,
                                code: None,
                                file: None,
                                limit: 200,
                                offset: 0,
                                include_logs: false,
                            };
                            self.store.query_events(&params).ok().map(|(evts, _total)| {
                                evts.into_iter()
                                    .map(|e| serde_json::to_value(&e).unwrap_or_default())
                                    .collect()
                            })
                        };

                        let events_json = if errors_only {
                            filter_events_errors_only(&events_json)
                        } else {
                            events_json
                        };

                        // Enrich error/warning events with surrounding source context + hints
                        let enriched_events = {
                            let cwd_path = std::path::PathBuf::from(cwd.unwrap_or("."));
                            let evts = events_json.clone().unwrap_or_default();
                            let evts_fallback = evts.clone();
                            let tool_name =
                                detected_tool_clone.as_ref().map(|t| t.tool_name.clone());
                            tokio::task::spawn_blocking(move || {
                                Some(enrich_events(evts, &cwd_path, tool_name.as_deref()))
                            })
                            .await
                            .unwrap_or(Some(evts_fallback))
                        };

                        let project_context = {
                            let status_clone = info.status.clone();
                            let cwd_path_clone = cwd.map(std::path::PathBuf::from);
                            let events_clone = enriched_events.clone();
                            tokio::task::spawn_blocking(move || {
                                compute_enhanced_project_context(
                                    &status_clone,
                                    cwd_path_clone.as_deref(),
                                    events_clone.as_ref().unwrap_or(&vec![]),
                                )
                            })
                            .await
                            .unwrap_or(None)
                        };

                        // Persist enriched events back to store (context + hints)
                        // Uses merge strategy to preserve log events (Bug #1 fix)
                        if let Some(ref evts) = enriched_events {
                            let task_events: Vec<crate::ipc::TaskEvent> = evts
                                .iter()
                                .filter_map(|e| serde_json::from_value(e.clone()).ok())
                                .collect();
                            if !task_events.is_empty() {
                                if let Err(e) = store_for_enrichment
                                    .merge_enriched_events(&task_id, &task_events)
                                {
                                    tracing::warn!(
                                        "failed to persist enriched events for {}: {}",
                                        task_id,
                                        e
                                    );
                                }
                                let _ = store_for_enrichment.mark_enriched(&task_id);
                            }
                        }

                        // Compute and save agent_delivered_bytes for telemetry
                        let rc_msg = extract_root_cause(&enriched_events).and_then(|rc| {
                            rc.get("message").and_then(|v| v.as_str()).map(|s| s.to_string())
                        });
                        let rc_file = extract_root_cause(&enriched_events).and_then(|rc| {
                            rc.get("file").and_then(|v| v.as_str()).map(|s| s.to_string())
                        });
                        let git_diff = project_context.as_ref().and_then(|pc| {
                            pc.get("git_diff_stat").and_then(|v| v.as_str()).map(|s| s.to_string())
                        });

                        let delivered_bytes = calculate_agent_delivered_bytes(
                            false,
                            0,
                            rc_msg.as_deref(),
                            rc_file.as_deref(),
                            git_diff.as_deref(),
                        );
                        let _ = store_for_enrichment
                            .update_task_agent_delivered_bytes(&task_id, delivered_bytes);

                        Ok(RunResult {
                            task_id: task_id.clone(),
                            status: info.status.clone(),
                            exit_code: Some(info.exit_code),
                            duration_ms: Some(info.duration_ms),
                            error_count: Some(info.error_count),
                            warning_count: Some(info.warning_count),
                            raw_output: None,
                            short_command: false,
                            root_cause: extract_root_cause(&enriched_events),
                            project_context,
                        })
                    }
                    Err(_) => Ok(RunResult {
                        task_id,
                        status: TaskStatus::Failed,
                        exit_code: Some(-1),
                        duration_ms: None,
                        error_count: None,
                        warning_count: None,
                        raw_output: None,
                        short_command: false,
                        root_cause: None,
                        project_context: None,
                    }),
                }
            }
            None => unreachable!(),
        }
    }

    /// Zero-overhead fast path for short commands.
    ///
    /// Skips Store insert, parser session, EventBus — directly spawns, waits,
    /// and returns raw stdout. Security checks and audit logging still apply.
    async fn run_short(
        &self,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        env: Option<&HashMap<String, String>>,
    ) -> Result<RunResult> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let start = std::time::Instant::now();

        let cwd_path = cwd.map(std::path::PathBuf::from);
        let mut handle = pty::spawn_command(command, cwd_path.as_deref(), env).await?;
        let _pid = handle.pid; // captured for audit/debug, not exposed to agent

        let timeout_dur = tokio::time::Duration::from_millis(
            timeout_ms.unwrap_or(self.config.max_task_duration_ms),
        );

        // Collect both stdout and stderr (the `_source` tag is deliberately
        // ignored): short commands return one merged raw blob so a failing
        // `git push` / `ls missing` still surfaces its stderr diagnostics.
        let mut stdout_lines: Vec<String> = Vec::new();
        let timed_out = tokio::select! {
            _result = async {
                while let Some((_source, line)) = handle.output_rx.recv().await {
                    stdout_lines.push(line);
                }
            } => Ok(()),
            _ = tokio::time::sleep(timeout_dur) => {
                let _ = handle.force_kill();
                Err(())
            }
        };

        let exit_code = match handle.wait().await {
            Ok(code) => code.unwrap_or(-1),
            Err(_) => -1,
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let status = if timed_out.is_err() {
            TaskStatus::Timeout
        } else {
            match exit_code {
                0 => TaskStatus::Completed,
                _ => TaskStatus::Failed,
            }
        };

        let raw_output = stdout_lines.join("\n");

        // Audit log
        if let Some(ref audit) = self.audit_log {
            let _ = audit.log(&AuditEntry {
                timestamp: chrono::Utc::now(),
                task_id: task_id.clone(),
                command: command.to_string(),
                cwd: cwd.map(String::from),
                exit_code: Some(exit_code),
                blocked: false,
                reason: None,
            });
        }

        Ok(RunResult {
            task_id,
            status,
            exit_code: Some(exit_code),
            duration_ms: Some(duration_ms),
            error_count: None,
            warning_count: None,
            raw_output: Some(raw_output),
            short_command: true,
            root_cause: None,
            project_context: None,
        })
    }

    /// Kill a running task gracefully (SIGINT → SIGTERM → SIGKILL).
    pub async fn kill(&self, task_id: &str) -> Result<()> {
        let kill_tx = self.kill_registry.lock().await.remove(task_id);
        match kill_tx {
            Some(tx) => {
                // Signal the background task to initiate graceful kill
                let _ = tx.send(()).await;
                Ok(())
            }
            None => {
                // Task not found in registry — may have already finished.
                // Mark as killed in DB anyway.
                self.store.update_task(task_id, &TaskStatus::Killed, Some(-1), None)?;
                Ok(())
            }
        }
    }

    /// Number of loaded parser entries (builtin + user).
    pub fn parser_count(&self) -> usize {
        self.parser.parser_count()
    }

    /// Tail the most recent events of a task, or the last `lines` lines of
    /// the raw output when `format` is "raw" (`<store>/raw/<task_id>.txt`).
    pub async fn tail(&self, task_id: &str, lines: usize, format: &str) -> Result<Vec<String>> {
        // 0 = all lines (raw channel needs a way to fetch the whole output).
        let lines = if lines == 0 { usize::MAX } else { lines };
        if format == "raw" {
            let raw_path = self.store.store_dir().join("raw").join(format!("{task_id}.txt"));
            let content = match std::fs::read_to_string(&raw_path) {
                Ok(c) => c,
                Err(_) => return Ok(vec![]),
            };
            let all: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            let start = all.len().saturating_sub(lines);
            return Ok(all[start..].to_vec());
        }
        use crate::ipc::QueryParams;
        let params = QueryParams {
            task_id: Some(task_id.to_string()),
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: lines,
            offset: 0,
            include_logs: true,
        };
        let (events, _total) = self.store.query_events(&params)?;
        Ok(events.into_iter().map(|e| e.message).collect())
    }
}
