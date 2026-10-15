//! Target-side first installation and independent exact-closure switching.
use super::remote::{LocalArgs, validate_flake, validate_hostname};
use super::{NativeExecutor, ReceiptStore, ServiceIdentity, activate, reconcile};
use eyre::{Result, ensure};
use std::path::Path;

pub async fn run_local(
    args: &LocalArgs,
    id: u64,
    bootstrap: Option<(&str, &str, &Path)>,
    reconcile_only: bool,
) -> Result<()> {
    ensure!(
        args.peer_config.is_file(),
        "existing private peer configuration required; never generate bootstrap credentials"
    );
    let raw = std::fs::read_to_string(&args.peer_config)?;
    let table: toml::Table = toml::from_str(&raw)
        .map_err(|_| eyre::eyre!("invalid private peer TOML (details suppressed)"))?;
    let settings = table
        .get("settings")
        .ok_or_else(|| eyre::eyre!("peer TOML needs [settings]"))?;
    let peer: crate::config::client::ConfigInner = settings
        .clone()
        .try_into()
        .map_err(|_| eyre::eyre!("invalid private peer settings (details suppressed)"))?;
    let mut address = peer.address();
    if address.ip().is_unspecified() {
        address.set_ip(if address.is_ipv6() {
            std::net::Ipv6Addr::LOCALHOST.into()
        } else {
            std::net::Ipv4Addr::LOCALHOST.into()
        });
    }
    let executor = NativeExecutor {
        address,
        key: peer.key,
        operator_sudo: args.operator_sudo,
        output: None,
    };
    let store = ReceiptStore::open_async(args.activation_directory.clone()).await?;
    if let Some((flake, host, nix)) = bootstrap {
        validate_flake(flake)?;
        validate_hostname(host)?;
        let builder = super::build::NativeBuild::new(nix)?;
        ensure!(
            args.operator_sudo,
            "first install requires explicit --operator-sudo authority"
        );
        ensure!(
            !Path::new("/etc/systemd/system/coeusd.service").exists()
                && !Path::new("/Library/LaunchDaemons/org.nixos.coeusd.plist").exists(),
            "existing managed daemon requires an idle operator migration, not bootstrap"
        );
        ensure!(
            tokio::net::TcpStream::connect(address).await.is_err(),
            "daemon endpoint is already listening; bootstrap is first-install only"
        );
        ensure!(
            !store.directory().join(format!("{id}.toml")).exists(),
            "bootstrap receipt exists; inspect/reconcile, do not rebuild or replay"
        );
        let platform = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        };
        let receipt =
            super::build::build_bootstrap(&store, id, flake, host, platform, &builder).await?;
        let expected = receipt.expected;
        // Bootstrap only when no managed daemon exists. Existing deployments use the handoff path.
        ensure!(
            !Path::new("/run/current-system/coeus-service.toml").exists(),
            "Coeus already installed; use deploy then switch JOB"
        );
        validate_peer_layout(&args.peer_config, &raw, &expected, store.directory())?;
    }
    let pending = store.load_async(id).await?;
    validate_peer_layout(
        &args.peer_config,
        &raw,
        &pending.expected,
        store.directory(),
    )?;
    let receipt = if reconcile_only {
        reconcile(&store, id, &executor).await?
    } else {
        activate(&store, id, &executor).await?
    };
    println!("job {}: {:?}: {}", id, receipt.state, receipt.message);
    Ok(())
}
fn validate_peer_layout(
    path: &Path,
    raw: &str,
    expected: &ServiceIdentity,
    directory: &Path,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = path.metadata()?;
    ensure!(
        metadata.uid() == nix::unistd::geteuid().as_raw()
            && metadata.permissions().mode() & 0o077 == 0,
        "peer configuration must be private and owned by service user"
    );
    ensure!(
        std::fs::canonicalize(path)? == std::fs::canonicalize(&expected.config_file)?,
        "bootstrap requires existing service server config as peer config"
    );
    let table: toml::Table = toml::from_str(raw)
        .map_err(|_| eyre::eyre!("invalid private server TOML (details suppressed)"))?;
    let config: crate::config::server::ConfigInner = table
        .get("settings")
        .ok_or_else(|| eyre::eyre!("missing server settings"))?
        .clone()
        .try_into()
        .map_err(|_| eyre::eyre!("invalid private server settings (details suppressed)"))?;
    let ledger = if config.state_directory.is_absolute() {
        config.state_directory
    } else {
        expected.state_directory.join(config.state_directory)
    };
    ensure!(
        super::directory_for_ledger(&ledger)? == directory,
        "activation directory must be derived from actual service ledger path"
    );
    Ok(())
}
