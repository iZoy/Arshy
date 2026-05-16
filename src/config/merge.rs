//! Config merge — partial overwrite, env vars, CLI overrides.

use super::{Config, PartialConfig, CliOverrides};
use std::path::PathBuf;

/// Merge PartialConfig (from file) into Config. Only Some fields overwrite.
pub fn merge_partial(mut cfg: Config, partial: PartialConfig) -> Config {
    if let Some(d) = partial.daemon {
        if let Some(v) = d.socket_path { cfg.daemon.socket_path = v; }
        if let Some(v) = d.log_level { cfg.daemon.log_level = v; }
        if let Some(v) = d.log_format { cfg.daemon.log_format = v; }
        if let Some(v) = d.auto_start { cfg.daemon.auto_start = v; }
        if let Some(v) = d.max_task_duration_ms { cfg.daemon.max_task_duration_ms = v; }
        if let Some(v) = d.max_output_bytes { cfg.daemon.max_output_bytes = v; }
        if let Some(v) = d.kill_graceful_ms { cfg.daemon.kill_graceful_ms = v; }
        if let Some(v) = d.kill_force_ms { cfg.daemon.kill_force_ms = v; }
        if let Some(v) = d.max_concurrent_tasks { cfg.daemon.max_concurrent_tasks = v; }
        if let Some(v) = d.sandbox_mode { cfg.daemon.sandbox_mode = v; }
    }
    if let Some(s) = partial.store {
        if let Some(v) = s.db_path { cfg.store.db_path = v; }
        if let Some(v) = s.wal_mode { cfg.store.wal_mode = v; }
        if let Some(v) = s.integrity_check { cfg.store.integrity_check = v; }
        if let Some(v) = s.auto_prune { cfg.store.auto_prune = v; }
        if let Some(v) = s.prune_keep { cfg.store.prune_keep = v; }
        if let Some(v) = s.prune_older_than_days { cfg.store.prune_older_than_days = v; }
        if let Some(v) = s.backend { cfg.store.backend = v; }
    }
    if let Some(p) = partial.parser {
        if let Some(v) = p.dirs { cfg.parser.dirs = v; }
        if let Some(v) = p.hot_reload { cfg.parser.hot_reload = v; }
        if let Some(v) = p.fallback_to_raw { cfg.parser.fallback_to_raw = v; }
        if let Some(v) = p.default_priority { cfg.parser.default_priority = v; }
        if let Some(v) = p.coverage_warning_threshold { cfg.parser.coverage_warning_threshold = v; }
        if let Some(v) = p.version_cache_ttl_hours { cfg.parser.version_cache_ttl_hours = v; }
    }
    if let Some(n) = partial.notifications {
        if let Some(v) = n.enabled { cfg.notifications.enabled = v; }
        if let Some(v) = n.batch_interval_ms { cfg.notifications.batch_interval_ms = v; }
        if let Some(v) = n.max_batch_events { cfg.notifications.max_batch_events = v; }
        if let Some(v) = n.min_severity { cfg.notifications.min_severity = v; }
    }
    if let Some(m) = partial.mcp {
        if let Some(v) = m.client_detection_order { cfg.mcp.client_detection_order = v; }
    }
    if let Some(t) = partial.telemetry {
        if let Some(v) = t.enabled { cfg.telemetry.enabled = v; }
    }
    if let Some(s) = partial.security {
        if let Some(v) = s.blocked_patterns { cfg.security.blocked_patterns = v; }
        if let Some(v) = s.allowed_commands { cfg.security.allowed_commands = Some(v); }
        if let Some(v) = s.sandbox_paths { cfg.security.sandbox_paths = v; }
        if let Some(v) = s.access_level { cfg.security.access_level = v; }
        if let Some(v) = s.audit_log { cfg.security.audit_log = Some(v); }
    }
    cfg
}

/// Apply `ARSHY_<SECTION>_<KEY>` env vars to config.
pub fn apply_env(cfg: &mut Config) {
    // Daemon
    if let Ok(v) = std::env::var("ARSHY_DAEMON_SOCKET_PATH") {
        cfg.daemon.socket_path = PathBuf::from(v);
    }
    if let Ok(v) = std::env::var("ARSHY_DAEMON_LOG_LEVEL") {
        cfg.daemon.log_level = v;
    }
    if let Ok(v) = std::env::var("ARSHY_DAEMON_LOG_FORMAT") {
        cfg.daemon.log_format = v;
    }
    if let Ok(v) = std::env::var("ARSHY_DAEMON_AUTO_START") {
        cfg.daemon.auto_start = parse_bool(&v);
    }
    if let Ok(v) = std::env::var("ARSHY_DAEMON_MAX_CONCURRENT_TASKS") {
        if let Ok(n) = v.parse() { cfg.daemon.max_concurrent_tasks = n; }
    }
    // Store
    if let Ok(v) = std::env::var("ARSHY_STORE_DB_PATH") {
        cfg.store.db_path = PathBuf::from(v);
    }
    if let Ok(v) = std::env::var("ARSHY_STORE_WAL_MODE") {
        cfg.store.wal_mode = parse_bool(&v);
    }
    if let Ok(v) = std::env::var("ARSHY_STORE_PRUNE_KEEP") {
        if let Ok(n) = v.parse() { cfg.store.prune_keep = n; }
    }
    // Parser
    if let Ok(v) = std::env::var("ARSHY_PARSER_HOT_RELOAD") {
        cfg.parser.hot_reload = parse_bool(&v);
    }
    if let Ok(v) = std::env::var("ARSHY_PARSER_FALLBACK_TO_RAW") {
        cfg.parser.fallback_to_raw = parse_bool(&v);
    }
    // Notifications
    if let Ok(v) = std::env::var("ARSHY_NOTIFICATIONS_ENABLED") {
        cfg.notifications.enabled = parse_bool(&v);
    }
    if let Ok(v) = std::env::var("ARSHY_NOTIFICATIONS_MIN_SEVERITY") {
        cfg.notifications.min_severity = v;
    }
    // Security
    if let Ok(v) = std::env::var("ARSHY_SECURITY_ACCESS_LEVEL") {
        cfg.security.access_level = v;
    }
}

/// Apply CLI overrides (highest priority).
pub fn apply_cli(cfg: &mut Config, cli: &CliOverrides) {
    if let Some(ref v) = cli.log_level {
        cfg.daemon.log_level = v.clone();
    }
    if let Some(ref v) = cli.socket_path {
        cfg.daemon.socket_path = v.clone();
    }
    if let Some(ref v) = cli.db_path {
        cfg.store.db_path = v.clone();
    }
}

fn parse_bool(s: &str) -> bool {
    s.eq_ignore_ascii_case("true") || s == "1"
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::*;

    // ── merge_partial tests ─────────────────────────────────────────────────

    #[test]
    fn merge_partial_basic() {
        let cfg = Config::default();
        let partial = PartialConfig {
            daemon: Some(PartialDaemonConfig {
                log_level: Some("debug".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let merged = merge_partial(cfg, partial);
        assert_eq!(merged.daemon.log_level, "debug");
    }

    #[test]
    fn merge_no_override_preserves_default() {
        let cfg = Config::default();
        let partial = PartialConfig::default();
        let merged = merge_partial(cfg.clone(), partial);
        assert_eq!(merged.daemon.log_level, cfg.daemon.log_level);
    }

    #[test]
    fn merge_partial_daemon_all_fields() {
        let cfg = Config::default();
        let partial = PartialConfig {
            daemon: Some(PartialDaemonConfig {
                log_level: Some("trace".into()),
                log_format: Some("json".into()),
                auto_start: Some(false),
                max_task_duration_ms: Some(60_000),
                max_output_bytes: Some(5_000_000),
                kill_graceful_ms: Some(5000),
                kill_force_ms: Some(3000),
                max_concurrent_tasks: Some(8),
                ..Default::default()
            }),
            ..Default::default()
        };
        let merged = merge_partial(cfg, partial);
        assert_eq!(merged.daemon.log_level, "trace");
        assert_eq!(merged.daemon.log_format, "json");
        assert!(!merged.daemon.auto_start);
        assert_eq!(merged.daemon.max_task_duration_ms, 60_000);
        assert_eq!(merged.daemon.max_output_bytes, 5_000_000);
        assert_eq!(merged.daemon.kill_graceful_ms, 5000);
        assert_eq!(merged.daemon.kill_force_ms, 3000);
        assert_eq!(merged.daemon.max_concurrent_tasks, 8);
    }

    #[test]
    fn merge_partial_store_section() {
        let cfg = Config::default();
        let partial = PartialConfig {
            store: Some(PartialStoreConfig {
                wal_mode: Some(false),
                integrity_check: Some(false),
                auto_prune: Some(true),
                prune_keep: Some(500),
                prune_older_than_days: Some(7),
                ..Default::default()
            }),
            ..Default::default()
        };
        let merged = merge_partial(cfg, partial);
        assert!(!merged.store.wal_mode);
        assert!(!merged.store.integrity_check);
        assert!(merged.store.auto_prune);
        assert_eq!(merged.store.prune_keep, 500);
        assert_eq!(merged.store.prune_older_than_days, 7);
    }

    #[test]
    fn merge_partial_parser_section() {
        let cfg = Config::default();
        let partial = PartialConfig {
            parser: Some(PartialParserConfig {
                hot_reload: Some(false),
                fallback_to_raw: Some(false),
                default_priority: Some(100),
                coverage_warning_threshold: Some(0.5),
                version_cache_ttl_hours: Some(48),
                ..Default::default()
            }),
            ..Default::default()
        };
        let merged = merge_partial(cfg, partial);
        assert!(!merged.parser.hot_reload);
        assert!(!merged.parser.fallback_to_raw);
        assert_eq!(merged.parser.default_priority, 100);
        assert!((merged.parser.coverage_warning_threshold - 0.5).abs() < f64::EPSILON);
        assert_eq!(merged.parser.version_cache_ttl_hours, 48);
    }

    #[test]
    fn merge_partial_notifications_section() {
        let cfg = Config::default();
        let partial = PartialConfig {
            notifications: Some(PartialNotificationsConfig {
                enabled: Some(false),
                batch_interval_ms: Some(500),
                max_batch_events: Some(10),
                min_severity: Some("warning".into()),
            }),
            ..Default::default()
        };
        let merged = merge_partial(cfg, partial);
        assert!(!merged.notifications.enabled);
        assert_eq!(merged.notifications.batch_interval_ms, 500);
        assert_eq!(merged.notifications.max_batch_events, 10);
        assert_eq!(merged.notifications.min_severity, "warning");
    }

    // ── apply_cli tests ─────────────────────────────────────────────────────

    #[test]
    fn apply_cli_overrides() {
        let mut cfg = Config::default();
        let cli = CliOverrides {
            log_level: Some("trace".into()),
            socket_path: Some(PathBuf::from("/tmp/custom.sock")),
            db_path: Some(PathBuf::from("/tmp/custom.db")),
            config_path: None,
        };
        apply_cli(&mut cfg, &cli);
        assert_eq!(cfg.daemon.log_level, "trace");
        assert_eq!(cfg.daemon.socket_path, PathBuf::from("/tmp/custom.sock"));
        assert_eq!(cfg.store.db_path, PathBuf::from("/tmp/custom.db"));
    }

    #[test]
    fn apply_cli_partial_overrides() {
        let mut cfg = Config::default();
        let original_socket = cfg.daemon.socket_path.clone();
        let cli = CliOverrides {
            log_level: Some("debug".into()),
            ..Default::default()
        };
        apply_cli(&mut cfg, &cli);
        assert_eq!(cfg.daemon.log_level, "debug");
        assert_eq!(cfg.daemon.socket_path, original_socket);
    }

    // ── parse_bool tests ────────────────────────────────────────────────────

    #[test]
    fn parse_bool_values() {
        assert!(parse_bool("true"));
        assert!(parse_bool("True"));
        assert!(parse_bool("TRUE"));
        assert!(parse_bool("1"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool("no"));
        assert!(!parse_bool(""));
    }
}
