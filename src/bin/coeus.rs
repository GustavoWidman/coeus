use clap::Parser;
use coeus::{cli::main::MainCLIArgs, client::CoeusClient, config::ClientSettings, utils::log::Logger};
use colored::Colorize;
use eyre::Result;
use log::info;

#[tokio::main]
async fn main() -> Result<()> {
    let args = MainCLIArgs::parse();
    Logger::init(args.verbosity);

    let settings = ClientSettings::load(args.config.as_deref())?;
    let key = settings.key.load()?;
    let endpoint = settings.endpoint(&args.host);

    let client = CoeusClient::connect(&settings, &endpoint, &key).await?;
    let host = client.host();
    info!(
        "{} is {} {} running coeus v{}",
        host.hostname.to_string().magenta(),
        host.platform,
        host.system,
        host.version
    );

    Ok(())
}
