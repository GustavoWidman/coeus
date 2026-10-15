use super::{Receipt, ReceiptStore, ServiceIdentity};
use eyre::{Result, ensure};
use futures_util::future::BoxFuture;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

pub struct BuiltSystem {
    pub closure: PathBuf,
    pub identity: ServiceIdentity,
}
pub trait BuildExecutor: Send + Sync {
    fn build<'a>(
        &'a self,
        attribute: &'a str,
        root: &'a Path,
    ) -> BoxFuture<'a, Result<BuiltSystem>>;
}
pub struct NativeBuild {
    nix: PathBuf,
}
impl NativeBuild {
    pub(crate) fn new(nix: &Path) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            nix.is_absolute(),
            "--nix-binary must be an absolute target path"
        );
        let metadata = std::fs::metadata(nix)
            .map_err(|_| eyre::eyre!("--nix-binary does not exist on target"))?;
        ensure!(
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
            "--nix-binary must be an executable file on target"
        );
        Ok(Self {
            nix: nix.to_path_buf(),
        })
    }
}
impl BuildExecutor for NativeBuild {
    fn build<'a>(
        &'a self,
        attribute: &'a str,
        root: &'a Path,
    ) -> BoxFuture<'a, Result<BuiltSystem>> {
        Box::pin(async move {
            let mut command = tokio::process::Command::new(&self.nix);
            command
                .args(["build", attribute, "--out-link"])
                .arg(root)
                .args(["--extra-experimental-features", "nix-command flakes"])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            let mut process = crate::deploy::process::DeploymentProcess::spawn(&mut command)?;
            ensure!(
                tokio::time::timeout(Duration::from_secs(3600), process.child.wait())
                    .await??
                    .success(),
                "bootstrap build failed; nothing activated"
            );
            Ok(BuiltSystem {
                closure: std::fs::canonicalize(root)?,
                identity: ServiceIdentity::read(root)?,
            })
        })
    }
}
pub async fn build_bootstrap(
    store: &ReceiptStore,
    id: u64,
    flake: &str,
    host: &str,
    platform: &str,
    builder: &dyn BuildExecutor,
) -> Result<Receipt> {
    ensure!(
        id != 0 && !store.directory().join(format!("{id}.toml")).exists(),
        "bootstrap receipt exists or invalid ID; inspect/reconcile, never rebuild/replay"
    );
    let attribute = if platform == "darwin" {
        format!("{flake}#darwinConfigurations.{host}.system")
    } else {
        format!("{flake}#nixosConfigurations.{host}.config.system.build.toplevel")
    };
    let root = store.root(id);
    ensure!(
        std::fs::symlink_metadata(&root).is_err(),
        "unreceipted retained root exists; inspect rather than rebuilding"
    );
    let built = builder.build(&attribute, &root).await?;
    ensure!(
        built.identity.host == host && built.identity.platform == platform,
        "built target identity mismatch"
    );
    let store = store.clone();
    tokio::task::spawn_blocking(move || {
        super::switch::prepare_built(&store, id, built.closure, built.identity, None, None)
    })
    .await?
}
