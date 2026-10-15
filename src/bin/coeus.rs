use clap::Parser;
use coeus::{
    cli::main::MainCLIArgs,
    client,
    config::{client::default_client_path, client_config},
    utils::log::Logger,
};
use eyre::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args = MainCLIArgs::parse();
    // Attach stdout is a byte-exact terminal stream, never a logger target.
    if matches!(args.command, coeus::cli::main::ClientCommand::Attach { .. }) {
        Logger::init_attach();
    } else {
        Logger::init(args.verbosity);
    }

    use coeus::cli::main::ClientCommand;
    match &args.command {
        ClientCommand::Identity => {
            print!(
                "{}",
                toml::to_string(&coeus::activation::BinaryIdentity::current())?
            );
            return Ok(());
        }
        ClientCommand::Switch { job_id, remote } => {
            return coeus::activation::run_remote(remote, *job_id, None, false).await;
        }
        ClientCommand::Reconcile { job_id, remote } => {
            return coeus::activation::run_remote(remote, *job_id, None, true).await;
        }
        ClientCommand::Bootstrap {
            job_id,
            remote,
            flake,
            hostname,
            nix_binary,
        } => {
            return coeus::activation::run_remote(
                remote,
                *job_id,
                Some((flake, hostname, nix_binary.as_path())),
                false,
            )
            .await;
        }
        ClientCommand::ActivateLocal { job_id, local } => {
            return coeus::activation::run_local(local, *job_id, None, false).await;
        }
        ClientCommand::ReconcileLocal { job_id, local } => {
            return coeus::activation::run_local(local, *job_id, None, true).await;
        }
        ClientCommand::BootstrapLocal {
            job_id,
            local,
            flake,
            hostname,
            nix_binary,
        } => {
            return coeus::activation::run_local(
                local,
                *job_id,
                Some((flake, hostname, nix_binary.as_path())),
                false,
            )
            .await;
        }
        _ => {}
    }
    let path = match args.config {
        Some(path) => path,
        None => default_client_path()?,
    };
    let config = client_config(path)?;
    client::run(config, args.command, args.no_tty).await
}
