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

    // Native service startup replaces the Darwin shell trampoline. Checks happen
    // before immutable config loading, ledger access or bind. No defaults are created.
    #[cfg(unix)]
    let config_path = match (args.service_home, args.service_state_directory) {
        (Some(home), Some(state)) => coeus::service_startup::prepare(&home, &state, &args.config)?,
        (None, None) => args.config,
        _ => {
            eyre::bail!("service startup requires both home and state directory");
        }
    };
    #[cfg(not(unix))]
    let config_path = {
        eyre::ensure!(
            args.service_home.is_none() && args.service_state_directory.is_none(),
            "module-managed service startup requires Unix"
        );
        args.config
    };

    let config: ServerConfig = server_config(config_path).inspect_err(|e| {
        error!("failed to load server configuration: {}", e);
    })?;

    let server = CoeusServer::new(config).await.inspect_err(|e| {
        error!("failed to initialize server: {}", e);
    })?;

    server.run().await.inspect_err(|error| {
        error!("fatal server runtime error: {error}");
    })
}
