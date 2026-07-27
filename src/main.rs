use clap::Parser;
use std::path::PathBuf;

mod cli;
mod proxy;

#[derive(Parser, Debug)]
#[command(
    name = "arshy",
    version,
    about = "AI Agent native shell — structured output, auto-mode intelligence, command filtering",
    long_about = "Arshy is a structured shell execution layer for AI agents. It replaces raw Bash with typed, queryable command execution: smart sync for short commands, async structured output for long commands, and 37 built-in parsers for common build/test tools."
)]
pub struct Cli {
    /// Run as MCP stdio proxy (for Claude Code / Cursor integration).
    /// The proxy connects to the arshyd daemon via Unix socket and translates
    /// MCP JSON-RPC tool calls into arshy IPC commands.
    #[arg(long = "from-mcp", verbatim_doc_comment)]
    pub from_mcp: bool,

    /// Path to config file. If not set, defaults are used with env var overrides.
    #[arg(long = "config", verbatim_doc_comment)]
    pub config: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error). Overrides config file.
    #[arg(long = "log-level", verbatim_doc_comment)]
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
        /// Output format: pretty (terminal UI), json (raw JSON), auto (default)
        #[arg(long, default_value = "auto")]
        format: String,
        /// Only return error-severity events
        #[arg(long)]
        errors_only: bool,
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
    /// Diagnose Claude Code integration and show fix suggestions
    Doctor,
    /// Run parser benchmark across all builtin parsers
    Benchmark,
    /// Generate impact analysis report
    Analyze {
        /// Output format: pretty (terminal UI), json (raw JSON), auto (default)
        #[arg(long, default_value = "auto")]
        format: String,
    },
    /// Manage parsers
    Parser {
        #[command(subcommand)]
        action: ParserAction,
    },
    /// Manage shell integration hook
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Claude Code PreToolUse hook handler (reads stdin, outputs stdout)
    ClaudeHook,
}

#[derive(clap::Subcommand, Debug)]
pub enum HookAction {
    /// Install the shell integration wrapper
    Install,
    /// Uninstall the shell integration wrapper
    Uninstall,
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

#[derive(clap::Subcommand, Debug)]
pub enum ParserAction {
    /// Reload parsers from disk and show what changed
    Reload,
    /// List all loaded parsers
    List,
}

fn main() -> arshy_lib::Result<()> {
    // Check if we should act as a shell wrapper (invoked as sh, bash, or zsh)
    let args: Vec<String> = std::env::args().collect();
    let mut is_shell = None;
    let mut is_claude_hook = false;
    if let Some(exe_path) = args.first() {
        let exe_path_buf = std::path::Path::new(exe_path);
        if let Some(exe_name) = exe_path_buf.file_name().and_then(|n| n.to_str()) {
            if exe_name == "sh" || exe_name == "bash" || exe_name == "zsh" {
                is_shell = Some(exe_name.to_string());
            } else if exe_name == "claude-hook" {
                is_claude_hook = true;
            }
        }
    }

    if is_claude_hook {
        return cli::shell_wrapper::run_claude_hook();
    }

    if let Some(name) = is_shell {
        return cli::shell_wrapper::run_wrapper(&name, args);
    }

    let cli = Cli::parse();

    if cli.from_mcp {
        proxy::run(cli.config)
    } else {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        rt.block_on(cli::dispatch(cli))
    }
}
