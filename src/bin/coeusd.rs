use std::os::unix::fs::DirBuilderExt;

use clap::Parser;
use coeus::{
    adopt,
    cli::daemon::{DaemonCLIArgs, DaemonCommand},
    config::daemon_config,
    engine::Engine,
    platform::Machine,
    runner,
    server::CoeusServer,
    utils::log::Logger,
};
use colored::Colorize;
use env_logger::Target;
use eyre::{Result, WrapErr};
use log::{error, info};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<()> {
    let args = DaemonCLIArgs::parse();
    // adopt's stdout is the stream `coeus bootstrap` reads
    let target = match &args.command {
        Some(DaemonCommand::Adopt(_)) => Target::Stderr,
        _ => Target::Stdout,
    };
    Logger::init_to(args.verbosity, target);

    match &args.command {
        Some(DaemonCommand::Run { dir }) => runner::run(dir).await,
        Some(DaemonCommand::Probe { verdict }) => runner::probe(verdict).await,
        Some(DaemonCommand::Adopt(adopt)) => std::process::exit(adopt::run(adopt).await?),
        None => {
            info!(
                "starting coeusd {}",
                format!("v{}", env!("CARGO_PKG_VERSION")).magenta()
            );

            serve(&args)
                .await
                .inspect_err(|error| error!("coeusd stopped: {error:#}"))
        }
    }
}

async fn serve(args: &DaemonCLIArgs) -> Result<()> {
    // drawn attaches carry coeus's colored lines to the client's terminal
    colored::control::set_override(true);

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
