use clap::Parser;
use coeus::{
    cli::daemon::DaemonCLIArgs,
    config::{ServerConfig, server_config},
    server::CoeusServer,
    utils::log::Logger,
};
use colored::Colorize;
use eyre::Result;
use log::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    let args = DaemonCLIArgs::parse();
    Logger::init(args.verbosity);

    info!(
        "starting coeusd {}",
        format!("v{}", env!("CARGO_PKG_VERSION")).magenta()
    );

    let config: ServerConfig = server_config(args.config).inspect_err(|e| {
        error!("failed to load server configuration: {}", e);
    })?;

    let server = CoeusServer::new(config).await.inspect_err(|e| {
        error!("failed to initialize server: {}", e);
    })?;

    if let Err(e) = server.run().await {
        error!("fatal server runtime error: {}", e);
    }

    Ok(())
}
