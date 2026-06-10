//! Config schema — all config sections with serde + Default.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Helper defaults ──────────────────────────────────────────────────────────

fn default_socket() -> PathBuf {
    PathBuf::from("${XDG_DATA_HOME}/arshy/arshyd.sock")
}
fn default_db() -> PathBuf {
    PathBuf::from("${XDG_DATA_HOME}/arshy/arshy.db")
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
fn d_1000() -> usize {
    1000
}
fn d_30() -> u32 {
    30
}
fn d_50() -> u32 {
    50
}
fn d_03() -> f64 {
    0.3
}
fn d_24() -> u64 {
    24
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
fn default_clients() -> Vec<String> {
    vec!["claude-code".into(), "cursor".into(), "windsurf".into()]
}
fn default_blocked_patterns() -> Vec<String> {
    vec![
        // ── Filesystem destruction ─────────────────────────────
        r"rm\s+-rf\s+[/~]".into(),
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
        r"sudo\s+.*rm\s+(-[a-zA-Z]*[rRfF]){2,}".into(), // sudo rm -rf
        r"sudo\s+.*\b(dd|mkfs|fdisk|parted)\b".into(),  // sudo disk tools
        r"sudo\s+.*\bchmod\s+(-R\s+)?777\b".into(),     // sudo chmod 777
        r"sudo\s+su\b".into(),                          // sudo su (shell escape)
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
fn default_backend() -> String {
    "sqlite".into()
}
fn default_max_commands_per_second() -> f64 {
    10.0
}
fn default_burst() -> f64 {
    20.0
}

// ── Top-level ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    sandbox_mode: String,
});

partial_section!(PartialStoreConfig {
    db_path: PathBuf,
    wal_mode: bool,
    integrity_check: bool,
    auto_prune: bool,
    prune_keep: usize,
    prune_older_than_days: u32,
    backend: String,
});

partial_section!(PartialParserConfig {
    dirs: Vec<PathBuf>,
    hot_reload: bool,
    fallback_to_raw: bool,
    default_priority: u32,
    coverage_warning_threshold: f64,
    version_cache_ttl_hours: u64,
});

partial_section!(PartialNotificationsConfig {
    enabled: bool,
    batch_interval_ms: u64,
    max_batch_events: usize,
    min_severity: String,
});

partial_section!(PartialMcpConfig {
    client_detection_order: Vec<String>,
});

partial_section!(PartialTelemetryConfig { enabled: bool });

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
    /// Reserved: "none" | "process" | "container". Currently only "none" is implemented.
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
    #[serde(default = "default_db")]
    pub db_path: PathBuf,
    #[serde(default = "d_true")]
    pub wal_mode: bool,
    #[serde(default = "d_true")]
    pub integrity_check: bool,
    #[serde(default)]
    pub auto_prune: bool,
    #[serde(default = "d_1000")]
    pub prune_keep: usize,
    #[serde(default = "d_30")]
    pub prune_older_than_days: u32,
    /// Reserved: "sqlite" | "postgres" | "redis". Currently only "sqlite" is implemented.
    #[serde(default = "default_backend")]
    pub backend: String,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            db_path: default_db(),
            wal_mode: d_true(),
            integrity_check: d_true(),
            auto_prune: false,
            prune_keep: d_1000(),
            prune_older_than_days: d_30(),
            backend: default_backend(),
        }
    }
}

impl StoreConfig {
    pub fn expanded_db_path(&self) -> PathBuf {
        super::expand_path(&self.db_path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParserConfig {
    #[serde(default = "default_dirs")]
    pub dirs: Vec<PathBuf>,
    #[serde(default = "d_true")]
    pub hot_reload: bool,
    #[serde(default = "d_true")]
    pub fallback_to_raw: bool,
    #[serde(default = "d_50")]
    pub default_priority: u32,
    #[serde(default = "d_03")]
    pub coverage_warning_threshold: f64,
    #[serde(default = "d_24")]
    pub version_cache_ttl_hours: u64,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            dirs: default_dirs(),
            hot_reload: d_true(),
            fallback_to_raw: d_true(),
            default_priority: d_50(),
            coverage_warning_threshold: d_03(),
            version_cache_ttl_hours: d_24(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsConfig {
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default = "d_100")]
    pub batch_interval_ms: u64,
    #[serde(default = "d_50e")]
    pub max_batch_events: usize,
    #[serde(default = "default_info")]
    pub min_severity: String,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: d_true(),
            batch_interval_ms: d_100(),
            max_batch_events: d_50e(),
            min_severity: default_info(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default = "default_clients")]
    pub client_detection_order: Vec<String>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self { client_detection_order: default_clients() }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled: bool,
}

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
