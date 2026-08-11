use crate::deploy::options::DeployOptions;
use clap::Parser;
use log::LevelFilter;

#[derive(Parser, Debug)]
#[command(name = "coeus")]
pub struct MainCLIArgs {
    /// Sets the logger's verbosity level
    #[arg(short, long, value_name = "VERBOSITY", default_value_t = LevelFilter::Info)]
    pub verbosity: LevelFilter,

    #[command(flatten)]
    pub options: DeployOptions,
}
