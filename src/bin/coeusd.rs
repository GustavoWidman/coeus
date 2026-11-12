use std::os::unix::fs::DirBuilderExt;

use clap::Parser;
use coeus::{
    cli::daemon::DaemonCLIArgs, config::daemon_config, engine::Engine, platform::Machine,
    server::CoeusServer, utils::log::Logger,
};
use colored::Colorize;
use eyre::{Result, WrapErr};
use log::{error, info};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    let args = DaemonCLIArgs::parse();
    Logger::init(args.verbosity);

    info!(
        "starting coeusd {}",
        format!("v{}", env!("CARGO_PKG_VERSION")).magenta()
    );

    if let Err(error) = serve(&args).await {
        error!("coeusd stopped: {error:#}");
        return Err(error);
    }

    Ok(())
}

async fn serve(args: &DaemonCLIArgs) -> Result<()> {
    let config = daemon_config(&args.config)?;
    let settings = (**config).clone();
    let key = settings.key.load()?;

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&settings.state_dir)
        .wrap_err_with(|| format!("failed to create {}", settings.state_dir.display()))?;

    let machine = Machine::new(&settings)?;
    let restart = CancellationToken::new();
    let shutdown = CancellationToken::new();

    let engine = Engine::start(
        machine,
        settings.clone(),
        &args.config,
        restart.clone(),
        shutdown.clone(),
    )
    .await?;

    let server = CoeusServer::bind(settings.listen, &key, engine).await?;
    let served = server.run(restart).await;
    shutdown.cancel();
    served
}
