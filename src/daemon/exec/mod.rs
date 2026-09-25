//! Execution engine — spawns commands, manages task lifecycle, streams events.
//!
//! Execution modes:
//! - **sync**: Wait for completion, return full result (blocking).
//! - **async**: Return immediately with task_id, stream events via notifications.
//! - **auto**: Inspection commands get the raw fast path; parser-backed or
//!   lifecycle-sensitive commands get structured execution.

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
#[allow(unused_imports)]
pub(crate) use decision::{classify_carrier, is_short_command, is_short_command_with_route};
pub(crate) use enrich::select_primary_diagnostic;

use crate::ipc::{Task, TaskStatus};
use crate::ArshyError;
use crate::Result;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::{oneshot, Mutex as TokioMutex, Semaphore};

use super::bus::EventBus;
use super::ipc_handler::RunResult;
use super::parser::Engine;
use super::reference::ReferenceTable;
use super::security::{AuditEntry, AuditLog, CommandFilter, RateLimiter};
use super::store::Store;

type RunDedupCache = HashMap<String, (RunResult, Instant, usize)>;

#[derive(serde::Serialize)]
struct RunDedupRequest<'a> {
    command: &'a str,
    cwd: Option<&'a str>,
    timeout_ms: Option<u64>,
    mode: &'a str,
    parse_hint: Option<&'a str>,
    env: Option<BTreeMap<&'a str, &'a str>>,
    errors_only: bool,
    purpose: Option<&'a str>,
}

/// Core executor that owns the store, parser, and event bus.
pub struct Executor {
    store: Arc<Store>,
    parser: Arc<Engine>,
    event_bus: EventBus,
    config: ExecutorConfig,
    filter: CommandFilter,
    allowed_cwds: Vec<String>,
    access_level: String,
    audit_log: Option<Arc<AuditLog>>,
    /// Registry of running tasks' kill signal senders.
    kill_registry: Arc<TokioMutex<HashMap<String, tokio::sync::mpsc::Sender<()>>>>,
    rate_limiter: Arc<TokioMutex<RateLimiter>>,
    /// Bound on concurrent task processes (`daemon.max_concurrent_tasks`).
    /// The permit is held for both fast and structured paths so disabling the
    /// rate limiter cannot create an unbounded number of child processes.
    task_semaphore: Arc<Semaphore>,
    /// Short-lived cache of run results keyed by the proxy-injected
    /// `dedup_key` (MCP request id). Prevents a replayed `tools/call` after a
    /// connection blip from executing the same command twice.
    run_dedup: Arc<TokioMutex<RunDedupCache>>,
    /// Per-key single-flight locks close the race between the cache lookup and
    /// execution. Weak entries disappear once all callers for a key finish.
    run_dedup_locks: Arc<TokioMutex<HashMap<String, std::sync::Weak<TokioMutex<()>>>>>,
    /// Restricted error-code reference tables (docker/kubectl/aws exit
    /// codes, ...). Served on demand through `task/query`; never inlined.
    /// Reloadable at runtime (user tables hot-reload via file watcher).
    reference: Arc<RwLock<ReferenceTable>>,
}

/// Runtime configuration for task execution.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    pub max_task_duration_ms: u64,
    pub max_output_bytes: u64,
    pub kill_graceful_ms: u64,
    pub kill_force_ms: u64,
    pub max_concurrent_tasks: usize,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_task_duration_ms: 3_600_000, // 1 hour
            max_output_bytes: 10_485_760,    // 10 MB
            kill_graceful_ms: 3_000,
            kill_force_ms: 2_000,
            max_concurrent_tasks: 4,
        }
    }
}

/// How long a deduped run result is reused. Covers the proxy's reconnect +
/// replay window without deduping deliberate repeat commands (every MCP call
/// carries a fresh request id).
const RUN_DEDUP_TTL_SECS: u64 = 120;

/// Cap on dedup cache entries to bound memory.
const RUN_DEDUP_CACHE_MAX: usize = 256;

/// Bound cached response memory as well as entry count. A single oversized
/// result may exceed this budget, but it replaces all older entries so the
/// cache cannot retain several large raw outputs at once.
const RUN_DEDUP_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;

fn run_dedup_cache_key(key: &str, request: &RunDedupRequest<'_>) -> String {
    let fingerprint = serde_json::to_vec(request).unwrap_or_default();
    let digest = Sha256::digest(fingerprint);
    let fingerprint = digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    format!("{key}:{fingerprint}")
}

fn run_result_size(result: &RunResult) -> usize {
    std::mem::size_of::<RunResult>()
        .saturating_add(result.task_id.len())
        .saturating_add(result.raw_output.as_ref().map_or(0, String::len))
        .saturating_add(
            result.primary_diagnostic.as_ref().map_or(0, |value| value.to_string().len()),
        )
}

fn cache_run_result(
    cache: &mut RunDedupCache,
    key: String,
    result: RunResult,
    inserted_at: Instant,
) {
    cache.retain(|_, (_, at, _)| at.elapsed().as_secs() < RUN_DEDUP_TTL_SECS);
    let result_size = run_result_size(&result);
    let mut cached_bytes =
        cache.values().fold(0usize, |total, (_, _, bytes)| total.saturating_add(*bytes));

    while !cache.is_empty()
        && (cache.len() >= RUN_DEDUP_CACHE_MAX
            || cached_bytes.saturating_add(result_size) > RUN_DEDUP_CACHE_MAX_BYTES)
    {
        let oldest = cache.iter().min_by_key(|(_, (_, at, _))| *at).map(|(key, _)| key.clone());
        let Some(oldest) = oldest else { break };
        if let Some((_, _, bytes)) = cache.remove(&oldest) {
            cached_bytes = cached_bytes.saturating_sub(bytes);
        }
    }

    cache.insert(key, (result, inserted_at, result_size));
}

/// Cancellation-safe activity marker for a fast-path command. The proxy can
/// disconnect while a command is still running; a `Drop` guard guarantees the
/// idle watchdog is released even when that cancels the executor future.
struct FastActivityGuard(Arc<Store>);

impl FastActivityGuard {
    fn new(store: Arc<Store>) -> Self {
        store.begin_fast_activity();
        Self(store)
    }
}

impl Drop for FastActivityGuard {
    fn drop(&mut self) {
        self.0.end_fast_activity();
    }
}

impl Executor {
    /// Create an executor with the default security policy.
    ///
    /// Call [`Self::with_security`] to replace it with application-specific
    /// security settings before accepting requests.
    pub fn new(store: Arc<Store>, parser: Arc<Engine>, event_bus: EventBus) -> Self {
        // Keep the public library constructor safe by default. The daemon
        // replaces this with the user's configured policy via `with_security`.
        let default_security = crate::config::SecurityConfig::default();
        let filter = CommandFilter::from_config(&default_security)
            .expect("the built-in security policy must contain valid regular expressions");
        Self {
            store,
            parser,
            event_bus,
            config: ExecutorConfig::default(),
            filter,
            allowed_cwds: Vec::new(),
            access_level: "full".into(),
            audit_log: None,
            kill_registry: Arc::new(TokioMutex::new(HashMap::new())),
            rate_limiter: Arc::new(TokioMutex::new(RateLimiter::disabled())),
            task_semaphore: Arc::new(Semaphore::new(
                ExecutorConfig::default().max_concurrent_tasks,
            )),
            run_dedup: Arc::new(TokioMutex::new(HashMap::new())),
            run_dedup_locks: Arc::new(TokioMutex::new(HashMap::new())),
            reference: Arc::new(RwLock::new(ReferenceTable::default())),
        }
    }

    pub fn with_config(mut self, config: ExecutorConfig) -> Self {
        self.task_semaphore = Arc::new(Semaphore::new(config.max_concurrent_tasks.max(1)));
        self.config = config;
        self
    }

    pub fn with_reference(mut self, reference: Arc<ReferenceTable>) -> Self {
        self.reference = Arc::new(RwLock::new((*reference).clone()));
        self
    }

    pub fn with_security(mut self, config: &crate::config::SecurityConfig) -> Result<Self> {
        if !matches!(config.access_level.as_str(), "full" | "read-only") {
            return Err(ArshyError::Config(format!(
                "invalid security.access_level '{}': expected 'full' or 'read-only'",
                config.access_level
            )));
        }
        if config.rate_limit.enabled
            && (!config.rate_limit.burst.is_finite()
                || config.rate_limit.burst < 1.0
                || !config.rate_limit.max_commands_per_second.is_finite()
                || config.rate_limit.max_commands_per_second <= 0.0)
        {
            return Err(ArshyError::Config(
                "invalid security.rate_limit: burst must be finite and at least 1, and max_commands_per_second must be finite and greater than 0".into(),
            ));
        }
        self.filter = CommandFilter::from_config(config)?;
        self.allowed_cwds = config.allowed_cwds.clone();
        self.access_level = config.access_level.clone();

        // Initialize rate limiter from config
        let rate_limiter = if config.rate_limit.enabled {
            RateLimiter::new(config.rate_limit.burst, config.rate_limit.max_commands_per_second)
        } else {
            RateLimiter::disabled()
        };
        self.rate_limiter = Arc::new(TokioMutex::new(rate_limiter));

        Ok(self)
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
        let senders: Vec<_> = self.kill_registry.lock().await.values().cloned().collect();
        let count = senders.len();
        if count > 0 {
            tracing::info!("killing {} running task(s)", count);
        }
        for tx in senders {
            // Cancellation is best-effort and idempotent; don't block shutdown
            // if an earlier request already occupies the one-slot channel.
            let _ = tx.try_send(());
        }
    }

    /// Schedule a command for execution (public entry point).
    ///
    /// When `dedup_key` is provided (the proxy passes the MCP request id), a
    /// repeated call with the same key within the TTL returns the cached
    /// result instead of executing again — replay protection for the proxy's
    /// reconnect-and-replay path.
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
        dedup_key: Option<&str>,
    ) -> Result<RunResult> {
        let cache_key = dedup_key.map(|key| {
            let canonical_env = env.map(|values| {
                values.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect()
            });
            let request = RunDedupRequest {
                command,
                cwd,
                timeout_ms,
                mode,
                parse_hint,
                env: canonical_env,
                errors_only,
                purpose,
            };
            run_dedup_cache_key(key, &request)
        });
        let mut dedup_guard = None;
        if let Some(key) = cache_key.as_deref() {
            let cache = self.run_dedup.lock().await;
            if let Some((result, at, _)) = cache.get(key) {
                if at.elapsed().as_secs() < RUN_DEDUP_TTL_SECS {
                    return Ok(result.clone());
                }
            }
            drop(cache);

            let key_lock = {
                let mut locks = self.run_dedup_locks.lock().await;
                locks.retain(|_, lock| lock.strong_count() > 0);
                if let Some(existing) = locks.get(key).and_then(std::sync::Weak::upgrade) {
                    existing
                } else {
                    let created = Arc::new(TokioMutex::new(()));
                    locks.insert(key.to_string(), Arc::downgrade(&created));
                    created
                }
            };
            let guard = key_lock.lock_owned().await;

            // Another caller may have completed while this caller waited.
            let cache = self.run_dedup.lock().await;
            if let Some((result, at, _)) = cache.get(key) {
                if at.elapsed().as_secs() < RUN_DEDUP_TTL_SECS {
                    return Ok(result.clone());
                }
            }
            drop(cache);
            dedup_guard = Some(guard);
        }

        let result = self
            .run_inner(command, cwd, timeout_ms, mode, parse_hint, env, errors_only, purpose)
            .await?;

        if let Some(key) = cache_key {
            let mut cache = self.run_dedup.lock().await;
            cache_run_result(&mut cache, key, result.clone(), Instant::now());
        }

        drop(dedup_guard);

        Ok(result)
    }

    /// Schedule a command for execution.
    ///
    /// Mode behavior:
    /// - **sync**: Wait for completion, full structured path.
    /// - **async**: Return immediately with task_id, events stream via notifications.
    /// - **auto**: Read-only inspection gets the raw fast path; parser-backed
    ///   commands get structured sync with a 60s wait before async fallback.
    ///   Only commands exceeding 60s degrade to async (returns task_id).
    #[allow(clippy::too_many_arguments)]
    async fn run_inner(
        &self,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        mode: &str,
        parse_hint: Option<&str>,
        env: Option<&HashMap<String, String>>,
        _errors_only: bool,
        purpose: Option<&str>,
    ) -> Result<RunResult> {
        if !matches!(mode, "auto" | "sync" | "async") {
            return Err(ArshyError::Ipc(format!(
                "invalid execution mode '{}': expected auto, sync, or async",
                mode
            )));
        }

        // ── Rate limit check (always run) ───────────────────────────────────
        {
            let mut limiter = self.rate_limiter.lock().await;
            if !limiter.try_acquire() {
                // Commands can contain credentials or other user data. Keep
                // them out of process logs even when requests are rejected.
                tracing::warn!("rate limit exceeded for command request");
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
            let paths = self.allowed_cwds.clone();
            tokio::task::spawn_blocking(move || super::security::check_path(&cwd_owned, &paths))
                .await
                .map_err(|e| ArshyError::Ipc(format!("sandbox check panicked: {}", e)))?
        };
        check_result?;

        // ── CWD accessibility check & symlink fallback ──────────────────
        // Some macOS privacy configurations can restrict child access to
        // ~/Documents, ~/Desktop, and ~/Downloads. For those paths we try a
        // private symlink alias; this does not guarantee TCC access, and
        // ARSHY_CWD carries the requested path for tools that need it.
        let cwd_path = cwd.map(std::path::PathBuf::from);
        let mut effective_env = env.cloned();
        let (fallback_cwd, symlink_to_cleanup) = if let Some(ref real) = cwd_path {
            prepare_cwd(real, &mut effective_env)
        } else {
            (None, None)
        };
        let spawn_cwd = fallback_cwd.or_else(|| cwd_path.clone());
        let env_for_spawn = effective_env.as_ref().or(env);

        // Record activity for the idle deadline. Every command counts — raw
        // and structured — so a daemon that only served inspection commands
        // (which never touch the store) still idle-exits.
        self.store.mark_activity();
        // Q1 telemetry: execution carrier distribution (all commands; persisted
        // per-task for structured commands below).
        let carrier = classify_carrier(command);
        super::telemetry::record_carrier(carrier.as_str());

        let is_auto = mode == "auto";
        let is_explicit_sync = mode == "sync";
        // When the agent provides a parse_hint, it expects structured output —
        // bypass the zero-overhead short path to ensure parser processing.
        let has_hint = parse_hint.is_some();

        // Detect before choosing the execution path. Parser definitions own
        // command detection, so adding a TOML parser automatically opts its
        // commands into structured execution without editing Rust policy.
        let detected_tool = if parse_hint == Some("json") {
            // Historical explicit JSON mode: it forces the structured path,
            // while the pipeline's whole-output JSON layer performs parsing.
            // It is not a registry asset and therefore has no ParsedTool.
            None
        } else if let Some(hint) = parse_hint {
            Some(self.parser.get_by_name(hint).ok_or_else(|| {
                ArshyError::Ipc(format!(
                    "unknown parser '{}'; omit parse_hint for auto-detection",
                    hint
                ))
            })?)
        } else {
            self.parser.detect(command)
        };
        let is_short = is_short_command_with_route(
            command,
            detected_tool.is_some() || has_hint,
            if has_hint {
                Some("structured")
            } else {
                detected_tool.as_ref().map(|tool| tool.route.as_str())
            },
        );

        // `max_concurrent_tasks` is a daemon-wide process bound, not a
        // parser-only bound. Fast-path commands are cheaper, but they still
        // consume a permit so concurrent MCP clients cannot create an
        // unbounded number of child processes when rate limiting is disabled.
        let task_permit = self.task_semaphore.clone().try_acquire_owned().map_err(|_| {
            ArshyError::Ipc(format!(
                "too many concurrent tasks: limit {} reached (max_concurrent_tasks)",
                self.config.max_concurrent_tasks
            ))
        })?;

        // ── Auto + short (no hint) → zero-overhead fast path ──────────────
        if is_auto && is_short && !has_hint {
            let _permit = task_permit;
            super::telemetry::record_task_created();
            let spawn_cwd_str = spawn_cwd.as_ref().map(|p| p.to_string_lossy().to_string());
            let _activity = FastActivityGuard::new(self.store.clone());
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
        let _permit = task_permit;

        let tool = detected_tool;
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
            carrier: Some(carrier.as_str().to_string()),
        };
        self.store.insert_task(&task)?;
        // Auto + non-short → sync (wait for completion, with a 60s handoff timeout)
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
        let store_on_failure = store_for_enrichment.clone();
        let event_bus_on_failure = self.event_bus.clone();
        let symlink_cleanup = symlink_to_cleanup;
        tokio::spawn(async move {
            // Hold the concurrency permit for the whole task lifetime.
            let _permit = _permit;
            if let Err(e) = run_background(task).await {
                tracing::error!("background task failed: {}", e);
                // A pre-spawn failure used to leave the persisted task in
                // Running forever, which also disabled daemon idle exit. Keep
                // the lifecycle terminal even when execution never started.
                let _ = store_on_failure.update_task(
                    &task_id_cleanup,
                    &TaskStatus::Failed,
                    Some(-1),
                    Some(0),
                );
                store_on_failure.mark_activity();
                event_bus_on_failure.publish(crate::daemon::bus::BusEvent {
                    connection_id: 0,
                    kind: crate::daemon::bus::BusEventKind::TaskComplete {
                        task_id: task_id_cleanup.clone(),
                        status: "failed".into(),
                        exit_code: -1,
                        duration_ms: 0,
                    },
                });
                super::telemetry::record_task_completed(false);
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
                event_count: None,
                raw_output: None,
                short_command: false,
                primary_diagnostic: None,
                raw_output_bytes: None,
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
                                event_count: None,
                                raw_output: None,
                                short_command: false,
                                primary_diagnostic: None,
                                raw_output_bytes: None,
                            });
                        }
                    }
                } else {
                    rx.await.map_err(|_| ())
                };

                match completion {
                    Ok(info) => {
                        // Keep the complete diagnostics in the store. The proxy emits only the
                        // command output inline and exposes a task handle when either channel is
                        // truncated.
                        let event_summary = {
                            let params = crate::ipc::QueryParams {
                                task_id: Some(task_id.clone()),
                                event_type: None,
                                severity: None,
                                code: None,
                                file: None,
                                limit: 0,
                                offset: 0,
                                include_logs: false,
                            };
                            self.store
                                .query_events_with_summary(&params)
                                .map(|(_, summary)| summary)
                                .unwrap_or_default()
                        };

                        // Counts describe the complete visible event set, not
                        // just the first 200 events loaded for enrichment.
                        let response_error_count = event_summary.errors;
                        let response_warning_count = event_summary.warnings;
                        let response_event_count = event_summary.total;

                        // Diagnostic selection must inspect the complete error set. The output
                        // capture limit bounds the event file, while this query avoids making the
                        // representative diagnostic depend on pagination order.
                        let primary_diagnostic = {
                            let params = crate::ipc::QueryParams {
                                task_id: Some(task_id.clone()),
                                event_type: None,
                                severity: Some("error".into()),
                                code: None,
                                file: None,
                                limit: usize::MAX,
                                offset: 0,
                                include_logs: false,
                            };
                            self.store.query_events(&params).ok().and_then(|(events, _)| {
                                let values = Some(
                                    events
                                        .into_iter()
                                        .map(|event| {
                                            serde_json::to_value(event).unwrap_or_default()
                                        })
                                        .collect(),
                                );
                                select_primary_diagnostic(&values)
                            })
                        };

                        let raw_len = store_for_enrichment.get_task_raw_output_bytes(&task_id);
                        let raw_output =
                            store_for_enrichment.get_task_raw_output(&task_id).ok().flatten();
                        Ok(RunResult {
                            task_id: task_id.clone(),
                            status: info.status.clone(),
                            exit_code: Some(info.exit_code),
                            duration_ms: Some(info.duration_ms),
                            error_count: Some(response_error_count),
                            warning_count: Some(response_warning_count),
                            event_count: Some(response_event_count),
                            raw_output,
                            short_command: false,
                            primary_diagnostic,
                            raw_output_bytes: Some(raw_len),
                        })
                    }
                    Err(_) => Ok(RunResult {
                        task_id,
                        status: TaskStatus::Failed,
                        exit_code: Some(-1),
                        duration_ms: None,
                        error_count: None,
                        warning_count: None,
                        event_count: None,
                        raw_output: None,
                        short_command: false,
                        primary_diagnostic: None,
                        raw_output_bytes: None,
                    }),
                }
            }
            None => unreachable!(),
        }
    }

    /// Zero-overhead raw-output path for inspection commands.
    ///
    /// Skips Store insert, parser session, EventBus — directly spawns, waits,
    /// and returns stdout/stderr lines in reader arrival order. Security
    /// checks and audit logging still apply.
    async fn run_short(
        &self,
        command: &str,
        cwd: Option<&str>,
        timeout_ms: Option<u64>,
        env: Option<&HashMap<String, String>>,
    ) -> Result<RunResult> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let started_at = chrono::Utc::now().to_rfc3339();
        let start = std::time::Instant::now();

        let cwd_path = cwd.map(std::path::PathBuf::from);
        let mut handle = pty::spawn_command(command, cwd_path.as_deref(), env).await?;
        let _pid = handle.pid; // captured for audit/debug, not exposed to agent

        let effective_timeout = timeout_ms
            .map(|value| value.min(self.config.max_task_duration_ms))
            .unwrap_or(self.config.max_task_duration_ms);
        let deadline =
            tokio::time::Instant::now() + tokio::time::Duration::from_millis(effective_timeout);

        // Collect both stdout and stderr (the `_source` tag is deliberately
        // ignored): short commands return one merged raw blob so a failing
        // `git push` / `ls missing` still surfaces its stderr diagnostics.
        let mut stdout_lines: Vec<String> = Vec::new();
        let mut captured_bytes = 0u64;
        let mut total_bytes = 0u64;
        let mut truncated = false;
        let max_bytes = self.config.max_output_bytes;
        let timed_out = tokio::select! {
            _result = async {
                while let Some((_source, line, line_bytes)) = handle.output_rx.recv().await {
                    total_bytes = total_bytes.saturating_add(line_bytes);
                    if !truncated && captured_bytes.saturating_add(line_bytes) <= max_bytes {
                        captured_bytes = captured_bytes.saturating_add(line_bytes);
                        stdout_lines.push(line);
                    } else {
                        // Continue draining after the capture limit. Dropping
                        // the receiver can back-pressure the child's pipes and
                        // turn output truncation into an indefinite hang.
                        truncated = true;
                    }
                }
            } => Ok(()),
            _ = tokio::time::sleep_until(deadline) => {
                let _ = handle.force_kill();
                Err(())
            }
        };

        // Closing stdout/stderr does not mean the process exited. Keep the
        // original deadline active while reaping the shell as well.
        let wait_result = if timed_out.is_ok() {
            tokio::select! {
                result = handle.wait() => Some(result),
                _ = tokio::time::sleep_until(deadline) => None,
            }
        } else {
            None
        };
        let timed_out = timed_out.is_err() || wait_result.is_none();
        if wait_result.is_none() {
            let _ = handle.force_kill();
        }
        let exit_code = if timed_out {
            -2
        } else {
            match match wait_result {
                Some(result) => result,
                None => handle.wait().await,
            } {
                Ok(code) => code.unwrap_or(-1),
                Err(_) => -1,
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let status = if timed_out {
            TaskStatus::Timeout
        } else {
            match exit_code {
                0 => TaskStatus::Completed,
                _ => TaskStatus::Failed,
            }
        };

        let mut raw_output = stdout_lines.join("\n");
        let mut persist_captured_output = truncated;
        if truncated {
            if !raw_output.is_empty() {
                raw_output.push('\n');
            }
            raw_output.push_str(&format!("[output truncated at {} bytes]", max_bytes));
        }
        persist_captured_output |= raw_output.len() > crate::ipc::MCP_INLINE_OUTPUT_LIMIT_BYTES;
        if persist_captured_output {
            // A truncated fast-path result needs a real handle so callers can
            // retrieve the captured output later when the MCP inline response
            // is clipped. Bytes beyond the configured capture limit are
            // deliberately drained and discarded.
            self.store.insert_task(&crate::ipc::Task {
                task_id: task_id.clone(),
                command: command.to_string(),
                cwd: cwd.map(str::to_owned),
                status: status.clone(),
                exit_code: Some(exit_code),
                pid: None,
                parser_name: None,
                started_at,
                finished_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_ms: Some(duration_ms),
                events_count: 0,
                error_count: 0,
                purpose: None,
                carrier: None,
            })?;
            self.store.update_task_raw_output(&task_id, &raw_output)?;
            self.store.update_task_raw_output_bytes(&task_id, total_bytes)?;
        }

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
            event_count: None,
            raw_output: Some(raw_output),
            short_command: true,
            primary_diagnostic: None,
            raw_output_bytes: Some(total_bytes),
        })
    }

    /// Kill a running task gracefully (SIGINT → SIGTERM → SIGKILL).
    pub async fn kill(&self, task_id: &str) -> Result<()> {
        let stored = self
            .store
            .get_task(task_id)?
            .ok_or_else(|| ArshyError::TaskNotFound(task_id.to_string()))?;
        if stored.status.is_terminal() {
            return Err(ArshyError::Ipc(format!(
                "task {} is already {}",
                task_id,
                stored.status.as_str()
            )));
        }

        let kill_tx = self.kill_registry.lock().await.get(task_id).cloned();
        match kill_tx {
            Some(tx) => {
                // Keep the sender registered until the background task has
                // fully completed. This makes concurrent/repeated cancel
                // requests idempotent instead of making the second request
                // mistake an in-progress cancellation for a missing worker.
                match tx.try_send(()) {
                    Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(())) => Ok(()),
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => {
                        Err(ArshyError::Ipc(format!(
                            "task {} is no longer accepting cancellation",
                            task_id
                        )))
                    }
                }
            }
            None => {
                // A Running task without a registry entry is inconsistent but
                // cannot be controlled. Make it terminal so it does not pin
                // daemon idle exit forever.
                let current = self
                    .store
                    .get_task(task_id)?
                    .ok_or_else(|| ArshyError::TaskNotFound(task_id.to_string()))?;
                if current.status.is_terminal() {
                    return Err(ArshyError::Ipc(format!(
                        "task {} is already {}",
                        task_id,
                        current.status.as_str()
                    )));
                }
                self.store.update_task(task_id, &TaskStatus::Killed, Some(-1), None)?;
                self.store.mark_activity();
                Ok(())
            }
        }
    }

    /// Number of loaded parser entries (builtin + user).
    pub fn parser_count(&self) -> usize {
        self.parser.parser_count()
    }

    /// Look up restricted reference entries for a code, optionally scoped
    /// to one tool (the task's parser name). When a tool is given, other
    /// tools' entries for the same code are never returned.
    pub fn reference_entries(
        &self,
        code: &str,
        tool: Option<&str>,
    ) -> Option<Vec<super::reference::ReferenceEntry>> {
        let table = self.reference.read().unwrap_or_else(|e| e.into_inner());
        table.lookup(code, tool)
    }

    /// Reload reference tables from disk (builtin + user dir). Returns a
    /// short diff summary. Used by hot-reload and `parser reload`.
    pub fn reload_reference(&self) -> Result<String> {
        let new_table = ReferenceTable::load()?;
        let mut table = self
            .reference
            .write()
            .map_err(|_| crate::ArshyError::Other("reference lock poisoned".into()))?;
        let before = table.count();
        let after = new_table.count();
        *table = new_table;
        Ok(format!("reference tables: {} -> {} entries", before, after))
    }

    /// Tail the most recent events of a task, or the last `lines` lines of
    /// the raw output when `format` is "raw" (`<store>/raw/<task_id>.txt`).
    pub async fn tail(&self, task_id: &str, lines: usize, format: &str) -> Result<Vec<String>> {
        if self.store.get_task(task_id)?.is_none() {
            return Err(ArshyError::TaskNotFound(task_id.to_string()));
        }
        // 0 = all lines (raw channel needs a way to fetch the whole output).
        let lines = if lines == 0 { usize::MAX } else { lines };
        if format == "raw" {
            let raw_path = self.store.store_dir().join("raw").join(format!("{task_id}.txt"));
            let content = match std::fs::read_to_string(&raw_path) {
                Ok(content) => content,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let expected_bytes = self.store.get_task_raw_output_bytes(task_id);
                    if expected_bytes == 0 {
                        return Ok(vec![]);
                    }
                    return Err(ArshyError::Exec(format!(
                        "raw output for task {task_id} is missing ({expected_bytes} bytes recorded)"
                    )));
                }
                Err(error) => return Err(error.into()),
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
