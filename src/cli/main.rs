use std::path::PathBuf;

use clap::Parser;
use log::LevelFilter;

use crate::common::proto::DeployRequest;

#[derive(Parser, Debug)]
#[command(name = "coeus")]
pub struct MainCLIArgs {
    /// Sets the logger's verbosity level
    #[arg(short, long, value_name = "VERBOSITY", default_value_t = LevelFilter::Info)]
    pub verbosity: LevelFilter,

    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE", default_value = "config.client.toml")]
    pub config: PathBuf,

    #[command(flatten)]
    pub options: DeployRequest,
}
