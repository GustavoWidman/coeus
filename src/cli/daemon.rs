use std::path::PathBuf;

use clap::{Parser, Subcommand};
use log::LevelFilter;

#[derive(Parser, Debug)]
#[command(name = "coeusd")]
pub struct DaemonCLIArgs {
    /// Sets the logger's verbosity level
    #[arg(short, long, value_name = "VERBOSITY", default_value_t = LevelFilter::Info)]
    pub verbosity: LevelFilter,

    /// Path to the configuration file
    #[arg(
        short,
        long,
        value_name = "FILE",
        default_value = "/etc/coeus/coeusd.toml"
    )]
    pub config: PathBuf,

    #[command(subcommand)]
    pub command: Option<DaemonCommand>,
}

/// what coeusd starts itself as, through the service manager
#[derive(Subcommand, Debug)]
pub enum DaemonCommand {
    /// Carries out the job in a job directory
    #[command(hide = true)]
    Run { dir: PathBuf },

    /// Writes whether this process has Full Disk Access to a file
    #[command(hide = true)]
    Probe { verdict: PathBuf },
}
