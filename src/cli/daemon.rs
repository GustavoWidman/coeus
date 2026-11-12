use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use log::LevelFilter;

use crate::common::model::{Hostname, NarHash, OrderId, StorePath};

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

    /// Admits a machine's first switch while no coeusd runs on it
    #[command(hide = true)]
    Adopt(AdoptArgs),
}

/// the one interface between `coeus bootstrap` and the coeus a machine's
/// configuration pins, before any coeusd runs there: every flag here must
/// keep its meaning across versions
#[derive(Args, Debug)]
pub struct AdoptArgs {
    /// The snapshot's store path, shipped in by `coeus bootstrap`
    #[arg(long, value_name = "STORE_PATH", required_unless_present = "status")]
    pub source: Option<StorePath>,

    /// The snapshot's NAR hash, e.g. sha256-…
    #[arg(long, value_name = "SRI")]
    pub nar_hash: NarHash,

    /// Where the snapshot came from, as history shows it
    #[arg(long, required_unless_present = "status")]
    pub label: Option<String>,

    /// This machine's name in the flake
    #[arg(long, required_unless_present = "status")]
    pub hostname: Option<Hostname>,

    /// The order to admit, one of the snapshot's bootstrap orders
    #[arg(long, value_name = "ORDER", required_unless_present = "status")]
    pub order: Option<OrderId>,

    /// Install the boot loader while activating
    #[arg(long)]
    pub install_bootloader: bool,

    /// Print the job's events from this log offset until it finishes
    #[arg(long, value_name = "OFFSET")]
    pub follow: Option<u64>,

    /// Print where the snapshot's switch stands, as JSON, and admit nothing
    #[arg(long, conflicts_with_all = ["source", "label", "hostname", "order", "install_bootloader", "follow"])]
    pub status: bool,
}
