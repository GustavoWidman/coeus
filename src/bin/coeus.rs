use clap::Parser;
use coeus::{
    cli::main::MainCLIArgs,
    client::CoeusClient,
    config::{ClientConfig, client_config},
    utils::log::Logger,
};
use eyre::Result;
use log::error;

#[tokio::main]
async fn main() -> Result<()> {
    let args = MainCLIArgs::parse();
    Logger::init(args.verbosity);

    let config: ClientConfig = client_config(args.config).inspect_err(|e| {
        error!("failed to load server configuration: {}", e);
    })?;

    let mut client = CoeusClient::new(config).await.inspect_err(|e| {
        error!("failed to initialize server: {}", e);
    })?;

    client.send_and_listen(args.options).await?;

    Ok(())
}
