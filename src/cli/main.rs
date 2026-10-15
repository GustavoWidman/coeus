use crate::common::proto::JobId;
use clap::{Args, Parser, Subcommand, ValueEnum};
use log::LevelFilter;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "coeus")]
pub struct MainCLIArgs {
    /// Sets the logger's verbosity level
    #[arg(short, long, value_name = "VERBOSITY", default_value_t = LevelFilter::Info)]
    pub verbosity: LevelFilter,
    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Dispatch only: never attach, prompt, or open a fleet TUI.
    #[arg(long, global = true)]
    pub no_tty: bool,
    #[command(subcommand)]
    pub command: ClientCommand,
}

#[derive(Subcommand, Debug)]
pub enum ClientCommand {
    /// Print exact binary version/protocol/OS/arch, without config or network access.
    Identity,
    /// Independently activate a retained daemon-changing build through verified SSH.
    Switch {
        job_id: JobId,
        #[command(flatten)]
        remote: crate::activation::RemoteArgs,
    },
    /// Verify an interrupted activation without replaying any switch command.
    Reconcile {
        job_id: JobId,
        #[command(flatten)]
        remote: crate::activation::RemoteArgs,
    },
    /// First installation via SSH; requires Nix, existing private config and operator privilege.
    Bootstrap {
        job_id: JobId,
        #[command(flatten)]
        remote: crate::activation::RemoteArgs,
        #[arg(long)]
        flake: String,
        #[arg(long)]
        hostname: String,
        /// Absolute target Nix executable; SSH shells need not have Nix on PATH.
        #[arg(long)]
        nix_binary: PathBuf,
    },
    #[command(hide = true)]
    ActivateLocal {
        job_id: JobId,
        #[command(flatten)]
        local: crate::activation::LocalArgs,
    },
    #[command(hide = true)]
    ReconcileLocal {
        job_id: JobId,
        #[command(flatten)]
        local: crate::activation::LocalArgs,
    },
    #[command(hide = true)]
    BootstrapLocal {
        job_id: JobId,
        #[command(flatten)]
        local: crate::activation::LocalArgs,
        #[arg(long)]
        flake: String,
        #[arg(long)]
        hostname: String,
        /// Absolute target Nix executable; SSH shells need not have Nix on PATH.
        #[arg(long)]
        nix_binary: PathBuf,
    },
    /// Deploy one target, or every configured fleet member when omitted.
    Deploy(DeployArgs),
    /// Show fleet activity or detailed status for one target (never a TUI).
    Status {
        target: Option<String>,
        #[arg(long)]
        watch: bool,
        /// Inspect a historical job on a single explicit target.
        #[arg(long, requires = "target")]
        job: Option<JobId>,
    },
    /// Follow shared raw daemon output across jobs. Ctrl-C only detaches.
    Attach {
        target: String,
        #[arg(long, conflicts_with = "after")]
        live: bool,
        #[arg(long)]
        after: Option<u64>,
    },
    /// Fetch compact durable plain-text logs (not a terminal attachment).
    Logs {
        job_id: JobId,
        /// Target machine; may be omitted only for a one-member fleet.
        #[arg(long)]
        target: Option<String>,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long)]
        follow: bool,
        /// Show log cursor/source metadata for durable inspection.
        #[arg(long)]
        metadata: bool,
        /// Filter retained records; merged PTY output has source terminal, NOT stderr.
        #[arg(long, value_enum)]
        source: Option<LogFilter>,
    },
    /// Request cancellation. Switching may already have modified the machine.
    Cancel {
        job_id: JobId,
        #[arg(long)]
        target: Option<String>,
    },
}

#[derive(Args, Debug, Clone)]
pub struct DeployArgs {
    pub target: Option<String>,
    /// Retry nonce (not the server-assigned job ID). Never blindly retry a lost ACK.
    #[arg(long, default_value_t = 0)]
    pub request_id: u64,
    /// Defaults to settings.default_revision, then main.
    #[arg(short, long)]
    pub rev: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub clean_substituters: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum LogFilter {
    Lifecycle,
    Stdout,
    Stderr,
    Terminal,
}
impl LogFilter {
    pub fn matches(self, source: crate::common::proto::LogSource) -> bool {
        use crate::common::proto::LogSource;
        matches!(
            (self, source),
            (Self::Lifecycle, LogSource::Lifecycle)
                | (Self::Stdout, LogSource::Stdout)
                | (Self::Stderr, LogSource::Stderr)
                | (Self::Terminal, LogSource::Terminal)
        )
    }
}
