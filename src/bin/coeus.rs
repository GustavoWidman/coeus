use clap::Parser;
use coeus::{cli::main::MainCLIArgs, client::CoeusClient, config::ClientSettings, utils::log::Logger};
use eyre::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args = MainCLIArgs::parse();
    Logger::init(args.verbosity);

    let settings = ClientSettings::load(args.config.as_deref())?;
    let key = settings.key.load()?;
    let endpoint = settings.endpoint(&args.host);

    let mut client = CoeusClient::connect(&settings, &endpoint, &key).await?;
    client.send(args.options).await
}
