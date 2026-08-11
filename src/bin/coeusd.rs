use clap::Parser;
use coeus::{
    cli::daemon::DaemonCLIArgs,
    utils::{
        config::{Config, config},
        log::Logger,
    },
};
use colored::Colorize;
use eyre::Result;
use log::info;

#[tokio::main]
async fn main() -> Result<()> {
    let args = DaemonCLIArgs::parse();
    Logger::init(args.verbosity);

    info!(
        "starting coeusd {}",
        format!("v{}", env!("CARGO_PKG_VERSION")).magenta()
    );

    let config: Config = config(args.config)?;
    // let manager = Manager::new(config).await?;

    // manager.run().await?;

    Ok(())
}
