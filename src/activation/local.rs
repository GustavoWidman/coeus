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
    // Identical immutable, permission-checked loading to daemon startup. No
    // defaults or generated credentials, including first-install bootstrap.
    let (peer_path, peer) = crate::config::server::load(&args.peer_config)?;
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
        validate_peer_layout(&peer_path, &peer, &expected, store.directory())?;
    }
    let pending = store.load_async(id).await?;
    validate_peer_layout(&peer_path, &peer, &pending.expected, store.directory())?;
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
    config: &crate::config::server::ConfigInner,
    expected: &ServiceIdentity,
    directory: &Path,
) -> Result<()> {
    ensure!(
        std::fs::canonicalize(path)? == std::fs::canonicalize(&expected.config_file)?,
        "bootstrap requires existing service server config as peer config"
    );
    let ledger = if config.state_directory.is_absolute() {
        config.state_directory.clone()
    } else {
        expected.state_directory.join(&config.state_directory)
    };
    ensure!(
        super::directory_for_ledger(&ledger)? == directory,
        "activation directory must be derived from actual service ledger path"
    );
    Ok(())
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[tokio::test]
    async fn independent_activation_validates_key_before_creating_receipt_directory() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let config = dir.path().join("peer.toml");
        let key = dir.path().join("missing-key");
        fs::write(
            &config,
            format!(
                "[settings]\nkey_file = {key:?}\nhost = \"127.0.0.1\"\nrepo_url = \"https://example.invalid/nix\"\n"
            ),
        )?;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600))?;
        let args = LocalArgs {
            peer_config: config.clone(),
            activation_directory: dir.path().join("receipts"),
            operator_sudo: false,
        };
        let error = run_local(&args, 1, None, true)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot resolve Coeus key file"));
        assert!(!args.activation_directory.exists());
        fs::write(&key, "42".repeat(32))?;
        fs::set_permissions(&key, fs::Permissions::from_mode(0o400))?;
        // Missing receipt stops before native activation/health commands. Successful
        // secret loading here is the same path used by daemon startup.
        assert!(run_local(&args, 1, None, true).await.is_err());
        assert!(args.activation_directory.is_dir());
        assert!(!fs::read_to_string(config)?.contains(&"42".repeat(32)));
        Ok(())
    }
}
