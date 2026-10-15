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

    /// Validate service ownership/permissions before loading configuration (module-owned).
    #[arg(long, value_name = "DIRECTORY", requires = "service_state_directory")]
    pub service_home: Option<PathBuf>,

    /// Private service working directory, paired with --service-home.
    #[arg(long, value_name = "DIRECTORY", requires = "service_home")]
    pub service_state_directory: Option<PathBuf>,
}
