use crate::common::proto::{DeployRequest, JobId};
use clap::{Parser, Subcommand};
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
    /// Accept a deployment; disconnecting after acceptance does not cancel it.
    Deploy(DeployRequest),
    /// Inspect one job or all retained jobs. Nix progress is phase-based, not a percentage.
    Status {
        job_id: Option<JobId>,
        #[arg(long)]
        watch: bool,
    },
    /// Fetch bounded log history; follow is reconnectable cursor-based attachment.
    Logs {
        job_id: JobId,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long)]
        follow: bool,
    },
    /// Request cancellation. Switching may already have modified the machine.
    Cancel { job_id: JobId },
}
