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
    // Attach stdout is a byte-exact terminal stream, never a logger target.
    if matches!(args.command, coeus::cli::main::ClientCommand::Attach { .. }) {
        Logger::init_attach();
    } else {
        Logger::init(args.verbosity);
    }

    let config: ClientConfig = client_config(args.config).inspect_err(|e| {
        error!("failed to load client configuration: {}", e);
    })?;

    let mut client = CoeusClient::new(config).await.inspect_err(|e| {
        error!("failed to initialize client: {}", e);
    })?;

    client.run_command(args.command).await?;

    Ok(())
}
