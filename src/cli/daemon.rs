use std::path::PathBuf;

use clap::Parser;
use log::LevelFilter;

#[derive(Parser, Debug)]
#[command(name = "coeusd")]
pub struct DaemonCLIArgs {
    /// Sets the logger's verbosity level
    #[arg(short, long, value_name = "VERBOSITY", default_value_t = LevelFilter::Info)]
    pub verbosity: LevelFilter,

    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE", default_value = "config.server.toml")]
    pub config: PathBuf,
}
