use super::{ActivationState, Receipt, ReceiptStore, ServiceIdentity};
use crate::common::{
    proto::{DeployPhase, IdentityRequest, Packet, StatusRequest},
    stream::EncryptedStream,
};
use eyre::{Result, WrapErr, bail, ensure};
use futures_util::future::BoxFuture;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;

/// All host effects are injected in lifecycle tests.
pub trait ActivationExecutor: Send + Sync {
    fn current(&self) -> Result<PathBuf>;
    fn validate<'a>(&'a self, receipt: &'a Receipt) -> BoxFuture<'a, Result<()>>;
    fn switch<'a>(&'a self, receipt: &'a Receipt) -> BoxFuture<'a, Result<()>>;
    fn verify<'a>(&'a self, receipt: &'a Receipt) -> BoxFuture<'a, Result<()>>;
}

pub fn prepare(
    store: &ReceiptStore,
    job_id: u64,
    expected: ServiceIdentity,
    previous: Option<ServiceIdentity>,
    previous_process_id: Option<u32>,
) -> Result<Receipt> {
    ensure!(job_id != 0, "nonzero job ID required");
    ensure!(
        !store.directory().join(format!("{job_id}.toml")).exists(),
        "receipt already exists; inspect/reconcile instead of rebuilding"
    );
    let root = store.root(job_id);
    let closure =
        std::fs::canonicalize(&root).wrap_err("build did not retain an out-link GC root")?;
    prepare_built(
        store,
        job_id,
        closure,
        expected,
        previous,
        previous_process_id,
    )
}

pub(crate) fn prepare_built(
    store: &ReceiptStore,
    job_id: u64,
    closure: PathBuf,
    expected: ServiceIdentity,
    previous: Option<ServiceIdentity>,
    previous_process_id: Option<u32>,
) -> Result<Receipt> {
    ensure!(
        job_id != 0 && !store.directory().join(format!("{job_id}.toml")).exists(),
        "receipt already exists or ID invalid"
    );
    let root = store.root(job_id);
    validate_store_path(&closure)?;
    expected.validate()?;
    if let Some(old) = &previous {
        ensure!(
            old.host == expected.host && old.platform == expected.platform,
            "target identity changed"
        );
        ensure!(
            old.user == expected.user
                && old.account == expected.account
                && old.state_directory == expected.state_directory,
            "service account policy/UID/GID/state migration requires an independent operator migration"
        );
    }
    let receipt = Receipt {
        format: 1,
        job_id,
        closure,
        root,
        manual: previous.as_ref() != Some(&expected),
        expected,
        previous,
        state: ActivationState::Ready,
        previous_process_id,
        message: "built and retained; not activated".into(),
    };
    store.save(&receipt)?;
    Ok(receipt)
}

/// Save intent BEFORE any privileged effect. In-flight/uncertain receipts never re-execute.
pub async fn activate(
    store: &ReceiptStore,
    id: u64,
    executor: &dyn ActivationExecutor,
) -> Result<Receipt> {
    activate_deadline(store, id, executor, Duration::from_secs(900)).await
}
pub(crate) async fn activate_deadline(
    store: &ReceiptStore,
    id: u64,
    executor: &dyn ActivationExecutor,
    deadline: Duration,
) -> Result<Receipt> {
    let _operation = store.operation()?;
    let mut receipt = store.load_async(id).await?;
    if receipt.state == ActivationState::Verified {
        executor.verify(&receipt).await?;
        return Ok(receipt);
    }
    ensure!(
        receipt.state == ActivationState::Ready,
        "activation already attempted or cancelled; use reconcile (verification only), never blind replay"
    );
    executor.validate(&receipt).await?;
    receipt.state = ActivationState::Activating;
    receipt.message =
        "activation started; interruption requires verification-only reconciliation".into();
    store.save_async(&receipt).await?;
    let result = async {
        tokio::time::timeout(deadline, executor.switch(&receipt))
            .await
            .wrap_err("activation timed out; outcome uncertain")??;
        executor.verify(&receipt).await
    }
    .await;
    match result {
        Ok(()) => {
            receipt.state = ActivationState::Verified;
            receipt.message = "exact closure and authenticated replacement daemon verified".into();
        }
        Err(error) => {
            receipt.state = ActivationState::Uncertain;
            receipt.message = crate::utils::plain_text::bounded(&format!("{error:#}"), 1024);
            store.save_async(&receipt).await?;
            return Err(error);
        }
    }
    store.save_async(&receipt).await?;
    Ok(receipt)
}

/// A disconnected switch may already have succeeded. Only observe; never run activation here.
pub async fn reconcile(
    store: &ReceiptStore,
    id: u64,
    executor: &dyn ActivationExecutor,
) -> Result<Receipt> {
    let _operation = store.operation()?;
    let mut receipt = store.load_async(id).await?;
    ensure!(
        matches!(
            receipt.state,
            ActivationState::Activating | ActivationState::Uncertain | ActivationState::Verified
        ),
        "receipt has not attempted activation"
    );
    executor.verify(&receipt).await?;
    receipt.state = ActivationState::Verified;
    receipt.message =
        "reconciled by exact closure and authenticated daemon observation; no activation replay"
            .into();
    store.save_async(&receipt).await?;
    Ok(receipt)
}

pub struct NativeExecutor {
    pub address: SocketAddr,
    pub key: [u8; 32],
    /// Explicit operator authority, needed before module sudo policy exists. No rules are installed.
    pub operator_sudo: bool,
}
impl NativeExecutor {
    async fn privileged(&self, receipt: &Receipt, args: &[String]) -> Result<()> {
        let mut command = if self.operator_sudo {
            let sudo = if receipt.expected.platform == "darwin" {
                "/usr/bin/sudo"
            } else {
                "/run/wrappers/bin/sudo"
            };
            let mut command = Command::new(sudo);
            command.args(["-n", "--"]);
            command.args(args);
            command
        } else {
            let previous = receipt.previous.as_ref().ok_or_else(|| {
                eyre::eyre!("bootstrap requires explicit --operator-sudo authority")
            })?;
            let mut command = Command::new(&previous.elevation_adapter);
            command.arg("env").args(args);
            command
        };
        command
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut process = crate::deploy::process::DeploymentProcess::spawn(&mut command)?;
        let status = process.child.wait().await?;
        ensure!(
            status.success(),
            "exact-closure activation command failed: {status}"
        );
        Ok(())
    }
}
impl ActivationExecutor for NativeExecutor {
    fn current(&self) -> Result<PathBuf> {
        Ok(std::fs::canonicalize("/run/current-system")?)
    }
    fn validate<'a>(&'a self, receipt: &'a Receipt) -> BoxFuture<'a, Result<()>> {
        let copy = receipt.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || validate_native(&copy)).await??;
            if receipt.manual && receipt.previous.is_some() {
                tokio::time::timeout(Duration::from_secs(10), async {
                    let mut stream = EncryptedStream::connect(self.address, &self.key).await?;
                    stream
                        .send(StatusRequest {
                            job_id: Some(receipt.job_id),
                            after: None,
                        })
                        .await?;
                    let Some(Packet::StatusResponse(status)) = stream.recv().await? else {
                        bail!("manual handoff status unavailable");
                    };
                    ensure!(
                        status.jobs.len() == 1
                            && status.jobs[0].phase == DeployPhase::RequiresManualSwitch,
                        "daemon has not durably handed off this job"
                    );
                    Ok::<_, eyre::Report>(())
                })
                .await??;
            }
            Ok(())
        })
    }
    fn switch<'a>(&'a self, receipt: &'a Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            for args in activation_commands(receipt) {
                self.privileged(receipt, &args).await?;
            }
            Ok(())
        })
    }
    fn verify<'a>(&'a self, receipt: &'a Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            ensure!(
                self.current()? == receipt.closure,
                "active closure differs from retained build"
            );
            let expected = &receipt.expected;
            let expected_daemon = expected.daemon.clone();
            let executable =
                tokio::task::spawn_blocking(move || std::fs::canonicalize(expected_daemon))
                    .await??;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            let mut last = "daemon not reachable".to_owned();
            loop {
                let probe = tokio::time::timeout(Duration::from_secs(2), async {
                    let mut stream = EncryptedStream::connect(self.address, &self.key).await?;
                    stream.send(IdentityRequest {}).await?;
                    let Some(Packet::IdentityResponse(response)) = stream.recv().await? else {
                        bail!("unexpected daemon identity response");
                    };
                    verify_daemon_identity(receipt, &response.identity, &executable)?;
                    Ok::<_, eyre::Report>(())
                })
                .await;
                match probe {
                    Ok(Ok(())) => return Ok(()),
                    Ok(Err(e)) => last = format!("{e:#}"),
                    Err(_) => {}
                }
                if tokio::time::Instant::now() >= deadline {
                    bail!("post-switch verification failed: {last}");
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
    }
}

pub(crate) fn activation_commands(receipt: &Receipt) -> Vec<Vec<String>> {
    let closure = receipt.closure.to_string_lossy();
    let profile = vec![
        receipt.expected.nix.to_string_lossy().into_owned(),
        "build".into(),
        "--no-link".into(),
        "--profile".into(),
        "/nix/var/nix/profiles/system".into(),
        closure.to_string(),
    ];
    let mut commands = Vec::new();
    if receipt.expected.platform == "linux" {
        // nh 4.4.2 contract: test -> profile -> boot. No reevaluation/rebuild on switch.
        commands.push(vec![
            format!("{closure}/bin/switch-to-configuration"),
            "test".into(),
        ]);
        commands.push(profile);
        commands.push(vec![
            format!("{closure}/bin/switch-to-configuration"),
            "boot".into(),
        ]);
        if receipt.manual {
            commands.push(vec![
                format!("{closure}/sw/bin/systemctl"),
                "restart".into(),
                "coeusd.service".into(),
            ]);
        }
    } else {
        commands.push(profile);
        commands.push(vec![
            format!("{closure}/sw/bin/darwin-rebuild"),
            "activate".into(),
        ]);
    }
    // Darwin activation reloads the changed launchd plist. Verification requires a new PID.
    commands
}
fn validate_store_path(path: &Path) -> Result<()> {
    ensure!(
        path.parent() == Some(Path::new("/nix/store")),
        "expected direct Nix store closure"
    );
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| eyre::eyre!("non-UTF8 store path"))?;
    ensure!(
        name.len() > 33
            && name.as_bytes()[32] == b'-'
            && name[..32]
                .bytes()
                .all(|b| b"0123456789abcdfghijklmnpqrsvwxyz".contains(&b)),
        "invalid store closure"
    );
    Ok(())
}

pub(crate) fn verify_daemon_identity(
    receipt: &Receipt,
    actual: &super::DaemonIdentity,
    executable: &Path,
) -> Result<()> {
    let expected = &receipt.expected;
    ensure!(
        actual.host == expected.host
            && actual.executable == executable
            && actual.runner == expected.runner,
        "replacement daemon identity mismatch"
    );
    let expected_os = if expected.platform == "darwin" {
        "macos"
    } else {
        "linux"
    };
    ensure!(
        actual.binary.protocol == expected.protocol
            && actual.binary.version == expected.version
            && actual.binary.os == expected_os
            && actual.binary.arch == std::env::consts::ARCH,
        "replacement daemon version/protocol/OS/arch mismatch"
    );
    ensure!(
        !receipt.manual || receipt.previous_process_id != Some(actual.process_id),
        "old daemon process still running"
    );
    Ok(())
}

fn validate_native(receipt: &Receipt) -> Result<()> {
    validate_store_path(&receipt.closure)?;
    ensure!(
        std::fs::canonicalize(&receipt.root)? == receipt.closure,
        "GC root changed"
    );
    ensure!(
        ServiceIdentity::read(&receipt.closure)? == receipt.expected,
        "retained closure service identity changed"
    );
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    ensure!(
        receipt.expected.platform == platform,
        "closure is for another platform"
    );
    let user = nix::unistd::User::from_uid(nix::unistd::geteuid())?
        .ok_or_else(|| eyre::eyre!("service account unavailable"))?
        .name;
    ensure!(
        user == receipt.expected.user,
        "switch/bootstrap must run as the service identity (state ownership is never migrated implicitly)"
    );
    ensure!(
        receipt
            .expected
            .account
            .uid
            .is_none_or(|uid| uid == nix::unistd::geteuid().as_raw())
            && receipt
                .expected
                .account
                .gid
                .is_none_or(|gid| gid == nix::unistd::getegid().as_raw()),
        "declared managed UID/GID differs from independent operator identity"
    );
    let host = nix::unistd::gethostname()?;
    ensure!(
        host.to_string_lossy().split('.').next() == Some(receipt.expected.host.as_str()),
        "host mismatch; refusing closure built for another machine"
    );
    if let Some(old) = &receipt.previous {
        ensure!(
            ServiceIdentity::read(Path::new("/run/current-system"))? == *old,
            "active service changed since build; do not switch stale handoff"
        );
    }
    Ok(())
}
