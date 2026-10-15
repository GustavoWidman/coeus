use crate::common::proto::{DeployRequest, JobId};
use clap::{Parser, Subcommand, ValueEnum};
use log::LevelFilter;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "coeus")]
pub struct MainCLIArgs {
    /// Sets the logger's verbosity level
    #[arg(short, long, value_name = "VERBOSITY", default_value_t = LevelFilter::Info)]
    pub verbosity: LevelFilter,
    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE", default_value = "config.client.toml")]
    pub config: PathBuf,
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
    /// Accept a deployment; disconnecting after acceptance does not cancel it.
    Deploy(DeployRequest),
    /// Inspect one job or all retained jobs. Nix progress is phase-based, not a percentage.
    Status {
        job_id: Option<JobId>,
        #[arg(long)]
        watch: bool,
    },
    /// Render raw terminal output. Ctrl-C detaches; it never cancels deployment.
    Attach {
        job_id: JobId,
        /// Skip replay and start at the live byte position.
        #[arg(long, conflicts_with = "after")]
        live: bool,
        /// Resume at an exact raw-byte cursor (not a log cursor).
        #[arg(long)]
        after: Option<u64>,
    },
    /// Fetch compact durable plain-text logs (not a terminal attachment).
    Logs {
        job_id: JobId,
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
    Cancel { job_id: JobId },
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
