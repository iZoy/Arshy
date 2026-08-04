//! Config schema — all config sections with serde + Default.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Helper defaults ──────────────────────────────────────────────────────────

fn default_socket() -> PathBuf {
    PathBuf::from("${XDG_DATA_HOME}/arshy/arshyd.sock")
}
fn default_store_dir() -> PathBuf {
    PathBuf::from("${XDG_DATA_HOME}/arshy")
}
fn default_info() -> String {
    "info".into()
}
fn default_text() -> String {
    "text".into()
}
fn d_true() -> bool {
    true
}
fn d_3600s() -> u64 {
    3_600_000
}
fn d_10mb() -> u64 {
    10_485_760
}
fn d_3s() -> u64 {
    3000
}
fn d_2s() -> u64 {
    2000
}
fn d_4() -> u32 {
    4
}
fn d_idle_timeout() -> u64 {
    // 15 minutes of inactivity before the daemon self-exits (0 = disabled).
    900
}
fn d_1000() -> usize {
    1000
}
fn d_30() -> u32 {
    30
}
fn d_100() -> u64 {
    100
}
fn d_50e() -> usize {
    50
}
fn default_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from("${HOME}/.arshy/parsers")]
}
fn default_blocked_patterns() -> Vec<String> {
    vec![
        // ── Filesystem destruction ─────────────────────────────
        r#"rm\s+-rf\s*(?:--\s*)?["']?[/~]"#.into(),
        r#"rm\s+--recursive\s+--force\s*["']?[/~]"#.into(),
        r"rm\s*\$\{IFS\}-rf".into(),
        r"dd\s+if=".into(),
        r"mkfs\.".into(),
        r"mkfs\s".into(),
        // ── Shell injection ───────────────────────────────────
        r"curl.*\|\s*(ba)?sh".into(),
        r"wget.*\|\s*(ba)?sh".into(),
        r"\|\s*(ba)?sh".into(),
        r"\|\s*base64\s+-d\s*\|\s*(ba)?sh".into(),
        r"base64\s+-d.*\|\s*(ba)?sh".into(),
        r"eval\s+\$\(|eval\s+`".into(),
        // ── Privilege escalation ──────────────────────────────
        r"sudo\s+.*rm\s+-[a-zA-Z]*[rR]".into(), // sudo rm -r (recursive)
        r"sudo\s+.*rm\s+-[a-zA-Z]*[fF]".into(), // sudo rm -f (force)
        r"sudo\s+.*\b(dd|mkfs|fdisk|parted)\b".into(), // sudo disk tools
        r"sudo\s+.*\bchmod\s+(-R\s+)?777\b".into(), // sudo chmod 777
        r"sudo\s+su\b".into(),                  // sudo su (shell escape)
        r"su\s+-".into(),
        // ── Credential exfiltration ──────────────────────────
        r"cat\s+.*\.ssh/(id_rsa|id_ed25519|id_dsa|id_ecdsa|authorized_keys)".into(),
        r"/proc/self/environ".into(),
        r"/proc/\d+/environ".into(),
        // ── Network abuse ────────────────────────────────────
        r"nc\s+-l".into(),
        r"ncat\s+-l".into(),
        // ── Dangerous permissions ────────────────────────────
        r"chmod\s+(-R\s+)?777".into(),
        // ── Fork bombs ───────────────────────────────────────
        r":\(\)\{\s*:\|:&\s*\};:".into(),
    ]
}
fn default_access_level() -> String {
    "full".into()
}
fn default_sandbox_mode() -> String {
    "none".into()
}
fn default_max_commands_per_second() -> f64 {
    10.0
}
fn default_burst() -> f64 {
    20.0
}

// ── Top-level ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Config format version. Incremented when the schema changes incompatibly.
    /// The current version is 1. Unknown versions trigger a warning.
    #[serde(default = "default_config_version")]
    pub version: u32,
    pub daemon: DaemonConfig,
    pub store: StoreConfig,
    pub parser: ParserConfig,
    pub notifications: NotificationsConfig,
    pub mcp: McpConfig,
    pub telemetry: TelemetryConfig,
    pub security: SecurityConfig,
}

fn default_config_version() -> u32 {
    1
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // The derived Default would set `version` to 0; the real config
            // version is 1 (kept in sync with default_config_version).
            version: default_config_version(),
            daemon: DaemonConfig::default(),
            store: StoreConfig::default(),
            parser: ParserConfig::default(),
            notifications: NotificationsConfig::default(),
            mcp: McpConfig::default(),
            telemetry: TelemetryConfig::default(),
            security: SecurityConfig::default(),
        }
    }
}

// ── Partial config (all optional — for file merge) ───────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PartialConfig {
    pub daemon: Option<PartialDaemonConfig>,
    pub store: Option<PartialStoreConfig>,
    pub parser: Option<PartialParserConfig>,
    pub notifications: Option<PartialNotificationsConfig>,
    pub mcp: Option<PartialMcpConfig>,
    pub telemetry: Option<PartialTelemetryConfig>,
    pub security: Option<PartialSecurityConfig>,
}

macro_rules! partial_section {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Debug, Default, Deserialize)]
        #[serde(default)]
        pub struct $name {
            $(pub $field: Option<$ty>),*
        }
    };
}

partial_section!(PartialDaemonConfig {
    socket_path: PathBuf,
    log_level: String,
    log_format: String,
    auto_start: bool,
    max_task_duration_ms: u64,
    max_output_bytes: u64,
    kill_graceful_ms: u64,
    kill_force_ms: u64,
    max_concurrent_tasks: u32,
    idle_timeout_secs: u64,
    sandbox_mode: String,
});

partial_section!(PartialStoreConfig {
    store_dir: PathBuf,
    integrity_check: bool,
    auto_prune: bool,
    prune_keep: usize,
    prune_older_than_days: u32,
});

partial_section!(PartialParserConfig {
    dirs: Vec<PathBuf>,
    hot_reload: bool,
});

partial_section!(PartialNotificationsConfig { batch_interval_ms: u64, max_batch_events: usize });

partial_section!(PartialMcpConfig {});

partial_section!(PartialTelemetryConfig {});

partial_section!(PartialSecurityConfig {
    blocked_patterns: Vec<String>,
    allowed_commands: Vec<String>,
    sandbox_paths: Vec<String>,
    access_level: String,
    audit_log: String,
    rate_limit: RateLimitConfig,
});

// ── Full config sections ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_socket")]
    pub socket_path: PathBuf,
    #[serde(default = "default_info")]
    pub log_level: String,
    #[serde(default = "default_text")]
    pub log_format: String,
    #[serde(default = "d_true")]
    pub auto_start: bool,
    #[serde(default = "d_3600s")]
    pub max_task_duration_ms: u64,
    #[serde(default = "d_10mb")]
    pub max_output_bytes: u64,
    #[serde(default = "d_3s")]
    pub kill_graceful_ms: u64,
    #[serde(default = "d_2s")]
    pub kill_force_ms: u64,
    #[serde(default = "d_4")]
    pub max_concurrent_tasks: u32,
    /// Self-exit after this many seconds of inactivity (no running tasks and the
    /// most recent finished task older than this). Prevents a daemon from
    /// lingering forever when the IDE is closed. Set to 0 to disable.
    #[serde(default = "d_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Sandbox mode: "none" (default) or "workspace" (lock execution to the
    /// daemon's working directory). "process"/"container" are not implemented.
    #[serde(default = "default_sandbox_mode")]
    pub sandbox_mode: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: default_socket(),
            log_level: default_info(),
            log_format: default_text(),
            auto_start: d_true(),
            max_task_duration_ms: d_3600s(),
            max_output_bytes: d_10mb(),
            kill_graceful_ms: d_3s(),
            kill_force_ms: d_2s(),
            max_concurrent_tasks: d_4(),
            idle_timeout_secs: d_idle_timeout(),
            sandbox_mode: default_sandbox_mode(),
        }
    }
}

impl DaemonConfig {
    pub fn expanded_socket_path(&self) -> PathBuf {
        super::expand_path(&self.socket_path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreConfig {
    /// Directory where Arshy persists its JSONL task/event files
    /// (`tasks.jsonl`, `events/<id>.jsonl`, `raw/<id>.txt`, `versions.json`).
    /// The store is a JSON-Lines file layout — **not** a SQLite database;
    /// the legacy `db_path`/`wal_mode`/`backend` fields were removed.
    /// Defaults to `${XDG_DATA_HOME}/arshy`.
    #[serde(default = "default_store_dir")]
    pub store_dir: PathBuf,
    #[serde(default = "d_true")]
    pub integrity_check: bool,
    #[serde(default)]
    pub auto_prune: bool,
    #[serde(default = "d_1000")]
    pub prune_keep: usize,
    #[serde(default = "d_30")]
    pub prune_older_than_days: u32,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            store_dir: default_store_dir(),
            integrity_check: d_true(),
            auto_prune: false,
            prune_keep: d_1000(),
            prune_older_than_days: d_30(),
        }
    }
}

impl StoreConfig {
    pub fn expanded_store_dir(&self) -> PathBuf {
        super::expand_path(&self.store_dir)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParserConfig {
    #[serde(default = "default_dirs")]
    pub dirs: Vec<PathBuf>,
    #[serde(default = "d_true")]
    pub hot_reload: bool,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self { dirs: default_dirs(), hot_reload: d_true() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    #[serde(default = "d_100")]
    pub batch_interval_ms: u64,
    #[serde(default = "d_50e")]
    pub max_batch_events: usize,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self { batch_interval_ms: d_100(), max_batch_events: d_50e() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetryConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_blocked_patterns")]
    pub blocked_patterns: Vec<String>,
    #[serde(default)]
    pub allowed_commands: Option<Vec<String>>,
    #[serde(default)]
    pub sandbox_paths: Vec<String>,
    #[serde(default = "default_access_level")]
    pub access_level: String,
    #[serde(default)]
    pub audit_log: Option<String>,
    #[serde(default)]
    pub rate_limit: RateLimitConfig,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            blocked_patterns: default_blocked_patterns(),
            allowed_commands: None,
            sandbox_paths: Vec::new(),
            access_level: default_access_level(),
            audit_log: None,
            rate_limit: RateLimitConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_commands_per_second")]
    pub max_commands_per_second: f64,
    #[serde(default = "default_burst")]
    pub burst: f64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_commands_per_second: default_max_commands_per_second(),
            burst: default_burst(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_socket_path_resolves_home() {
        let cfg = DaemonConfig {
            socket_path: std::path::PathBuf::from("${HOME}/.arshy/arshy.sock"),
            ..Default::default()
        };
        let expanded = cfg.expanded_socket_path();
        let home = dirs::home_dir().expect("home dir should exist");
        assert!(expanded.starts_with(home), "expanded={expanded:?}");
        assert!(expanded.ends_with("arshy.sock"));
    }

    #[test]
    fn expanded_socket_path_resolves_xdg_data_home() {
        let cfg = DaemonConfig {
            socket_path: std::path::PathBuf::from("${XDG_DATA_HOME}/arshy/arshy.sock"),
            ..Default::default()
        };
        let expanded = cfg.expanded_socket_path();
        assert!(!expanded.to_string_lossy().contains("${XDG_DATA_HOME}"));
        assert!(expanded.ends_with(std::path::Path::new("arshy/arshy.sock")));
    }

    #[test]
    fn expanded_socket_path_passthrough_when_no_placeholder() {
        let cfg = DaemonConfig {
            socket_path: std::path::PathBuf::from("/tmp/arshy.sock"),
            ..Default::default()
        };
        assert_eq!(cfg.expanded_socket_path(), std::path::PathBuf::from("/tmp/arshy.sock"));
    }

    #[test]
    fn expanded_store_dir_resolves_placeholder() {
        let cfg = StoreConfig {
            store_dir: std::path::PathBuf::from("${XDG_DATA_HOME}/arshy-data"),
            ..Default::default()
        };
        let expanded = cfg.expanded_store_dir();
        assert!(!expanded.to_string_lossy().contains("${XDG_DATA_HOME}"));
        assert!(expanded.ends_with("arshy-data"));
    }

    #[test]
    fn defaults_are_sane() {
        let d = DaemonConfig::default();
        assert_eq!(d.sandbox_mode, "none");
        assert!(d.max_concurrent_tasks > 0);
        let s = StoreConfig::default();
        assert!(s.prune_keep > 0);
        assert!(s.prune_older_than_days > 0);
        let p = ParserConfig::default();
        assert!(p.hot_reload);
    }
}
