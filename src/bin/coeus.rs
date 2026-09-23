use clap::Parser;
use coeus::{
    cli::main::MainCLIArgs,
    client::CoeusClient,
    common::proto::Packet,
    config::{ClientConfig, client_config},
    utils::log::Logger,
};
use eyre::{Result, eyre};
use log::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    let args = MainCLIArgs::parse();
    Logger::init(args.verbosity);

    let config: ClientConfig = client_config(args.config).inspect_err(|e| {
        error!("failed to load server configuration: {}", e);
    })?;

    let client = CoeusClient::new(config).await.inspect_err(|e| {
        error!("failed to initialize server: {}", e);
    })?;

    let (mut reader, mut writer) = client.split();
    let receive_response = async {
        reader
            .recv()
            .await?
            .ok_or_else(|| eyre!("server closed before acknowledging the deploy request"))
    };

    let (_, response) = tokio::try_join!(writer.send(args.options), receive_response)?;

    match response {
        Packet::DeployAccepted(accepted) => {
            info!("deployment request accepted: {}", accepted.message);
        }
        packet => return Err(eyre!("unexpected response packet: {packet:?}")),
    }

    Ok(())
}
