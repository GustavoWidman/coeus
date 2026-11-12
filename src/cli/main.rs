use std::path::PathBuf;

use clap::Parser;
use log::LevelFilter;

use crate::common::model::Hostname;

#[derive(Parser, Debug)]
#[command(name = "coeus")]
pub struct MainCLIArgs {
    /// Sets the logger's verbosity level
    #[arg(short, long, value_name = "VERBOSITY", default_value_t = LevelFilter::Info)]
    pub verbosity: LevelFilter,

    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE", env = "COEUS_CONFIG")]
    pub config: Option<PathBuf>,

    /// The machine to reach
    pub host: Hostname,
}
