use clap::Parser;
use coeus::{cli::main::MainCLIArgs, deploy::deploy, utils::log::Logger};
use eyre::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args = MainCLIArgs::parse();
    Logger::init(args.verbosity);

    deploy(&args.options).await?;

    Ok(())
}
