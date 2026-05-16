use clap::Parser;
use std::path::PathBuf;

mod cli;
mod proxy;

#[derive(Parser, Debug)]
#[command(name = "arshy", version, about = "AI Agent native shell execution layer")]
pub struct Cli {
    /// Run as MCP stdio proxy
    #[arg(long = "from-mcp")]
    pub from_mcp: bool,

    /// Path to config file
    #[arg(long = "config")]
    pub config: Option<PathBuf>,

    /// Log level
    #[arg(long = "log-level")]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(clap::Subcommand, Debug)]
pub enum CliCommand {
    /// Execute a shell command
    Run {
        command: String,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        timeout_ms: Option<u64>,
        #[arg(long)]
        mode: Option<String>,
    },
    /// List tasks
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Query events from a task
    Query {
        task_id: String,
        #[arg(long)]
        event_type: Option<String>,
        #[arg(long)]
        severity: Option<String>,
        #[arg(long)]
        code: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Kill a running task
    Kill { task_id: String },
    /// Tail task output
    Tail {
        task_id: String,
        #[arg(long, default_value = "50")]
        lines: usize,
        #[arg(long, default_value = "event")]
        format: String,
    },
    /// Register as MCP server for Claude Code / Cursor
    Install,
    /// Remove MCP registration
    Uninstall,
    /// Prune old task history
    Prune {
        #[arg(long)]
        keep: Option<usize>,
        #[arg(long)]
        older_than: Option<String>,
    },
    /// View or modify arshy configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Show daemon status
    Status,
    /// Manage the daemon process
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Show aggregate statistics
    Stats,
    /// Install macOS launchd plist for auto-start
    InstallLaunchd,
    /// Install Linux systemd user unit for auto-start
    InstallSystemd,
}

#[derive(clap::Subcommand, Debug)]
pub enum ConfigAction {
    /// Get a config value by key (e.g. "daemon.log_level")
    Get { key: String },
    /// Set a config value (e.g. set daemon.log_level debug)
    Set { key: String, value: String },
    /// List all config values
    List,
    /// Show the config file path
    Path,
}

#[derive(clap::Subcommand, Debug)]
pub enum DaemonAction {
    /// Start the daemon (if not already running)
    Start,
    /// Stop the running daemon
    Stop,
    /// Restart the daemon (stop + start)
    Restart,
}

fn main() -> arshy_lib::Result<()> {
    let cli = Cli::parse();

    if cli.from_mcp {
        proxy::run(cli.config)
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(cli::dispatch(cli))
    }
}
