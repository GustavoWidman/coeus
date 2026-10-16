//! Synthetic state and injected immutable identities. Never real services/deployers.
use super::*;
use crate::{
    activation::{AccountPolicy, ActivationExecutor},
    common::proto::DeployRequest,
    jobs::{DeploymentExecutor, DeploymentJobs, JobContext},
};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};

#[derive(Clone)]
struct FixtureEnvironment {
    installed: ServiceIdentity,
    closure: PathBuf,
}
impl Environment for FixtureEnvironment {
    fn installed(&self) -> Result<ServiceIdentity> {
        Ok(self.installed.clone())
    }
    fn retained(&self, receipt: &Receipt) -> Result<()> {
        ensure!(
            receipt.expected == self.installed && receipt.closure == self.closure,
            "fixture retained identity mismatch"
        );
        Ok(())
    }
}
struct Executor {
    env: FixtureEnvironment,
    started: mpsc::UnboundedSender<String>,
}
impl DeploymentExecutor for Executor {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if request.rev != "partial" {
                self.started.send(request.rev)?;
                return Ok(());
            }
            context.phase(DeployPhase::Switch).await?;
            let store = ReceiptStore::open(&context.activation_directory().await?)?;
            symlink(&self.env.closure, store.root(context.job_id()))?;
            store.save(&Receipt {
                format: 1,
                job_id: context.job_id(),
                closure: self.env.closure.clone(),
                root: store.root(context.job_id()),
                expected: self.env.installed.clone(),
                previous: Some(self.env.installed.clone()),
                state: ActivationState::Uncertain,
                manual: false,
                previous_process_id: None,
                message: "synthetic activation failed".into(),
                operator_resolution: None,
            })?;
            drop(store);
            self.started.send(request.rev)?;
            context.cancelled().await;
            eyre::bail!("synthetic cancelled partial activation");
        })
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    env: FixtureEnvironment,
    config: PathBuf,
    ledger: PathBuf,
    activation: PathBuf,
}
impl Fixture {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("service");
        fs::create_dir(&work)?;
        fs::set_permissions(&work, fs::Permissions::from_mode(0o700))?;
        let closure = dir.path().join("retained-system");
        fs::create_dir(&closure)?;
        let config = dir.path().join("server.toml");
        let ledger = work.join("state");
        fs::write(
            &config,
            format!(
                "[settings]\nkey='{}'\nhost='127.0.0.1'\nrepo_url='https://example.invalid/nix'\nstate_directory={ledger:?}\n",
                "42".repeat(32)
            ),
        )?;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600))?;
        let user = nix::unistd::User::from_uid(nix::unistd::geteuid())?
            .unwrap()
            .name;
        let installed = ServiceIdentity {
            host: "fixture".into(),
            platform: "linux".into(),
            daemon: "/nix/store/00000000000000000000000000000000-coeus/bin/coeusd".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: crate::common::proto::PROTOCOL_VERSION,
            user,
            account: AccountPolicy {
                managed: false,
                uid: None,
                gid: None,
            },
            state_directory: work,
            config_file: config.clone(),
            runner: "/nix/store/00000000000000000000000000000000-runner".into(),
            elevation_adapter: "/nix/store/00000000000000000000000000000000-adapter".into(),
            nix: "/nix/store/00000000000000000000000000000000-nix/bin/nix".into(),
            service_definition: "fixture service".into(),
        };
        Ok(Self {
            activation: directory_for_ledger(&ledger)?,
            _dir: dir,
            env: FixtureEnvironment { installed, closure },
            config,
            ledger,
        })
    }
    fn args(&self) -> RecoverArgs {
        RecoverArgs {
            server_config: self.config.clone(),
            confirm: None,
            reason: None,
            attest_no_writers: false,
            accept_partial_state: false,
        }
    }
    fn confirm(&self, preview: &str) -> RecoverArgs {
        let mut args = self.args();
        args.confirm = Some(
            preview
                .lines()
                .find_map(|l| l.strip_prefix("Challenge: "))
                .unwrap()
                .into(),
        );
        args.reason = Some("Stopped prior writers and inspected partial state".into());
        args.attest_no_writers = true;
        args.accept_partial_state = true;
        args
    }
    async fn jobs(&self) -> Result<(DeploymentJobs, mpsc::UnboundedReceiver<String>)> {
        let (started, receiver) = mpsc::unbounded_channel();
        Ok((
            DeploymentJobs::open(
                Arc::new(Executor {
                    env: self.env.clone(),
                    started,
                }),
                self.ledger.clone(),
                "https://example.invalid/nix".into(),
            )
            .await?,
            receiver,
        ))
    }
    async fn interrupted(&self) -> Result<(u64, u64)> {
        let (jobs, mut started) = self.jobs().await?;
        let first = jobs.accept(request("partial")).await?;
        started.recv().await;
        let second = jobs.accept(request("next")).await?;
        jobs.cancel(first).await?;
        phase(&jobs, first, DeployPhase::Interrupted).await?;
        jobs.shutdown().await;
        drop(jobs);
        Ok((first, second))
    }
}
fn request(rev: &str) -> DeployRequest {
    DeployRequest {
        request_id: rand::random::<u64>().max(1),
        rev: rev.into(),
        dry_run: false,
        clean_substituters: false,
    }
}
async fn phase(jobs: &DeploymentJobs, id: u64, expected: DeployPhase) -> Result<()> {
    timeout(Duration::from_secs(4), async {
        loop {
            if jobs.status(Some(id)).await?[0].phase == expected {
                return Ok::<_, eyre::Report>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await??;
    Ok(())
}
fn tree(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)?;
            if meta.is_dir() {
                visit(root, &path, out)?;
            } else if meta.file_type().is_symlink() {
                out.push((
                    path.strip_prefix(root)?.into(),
                    fs::read_link(&path)?.to_string_lossy().as_bytes().to_vec(),
                ));
            } else {
                out.push((path.strip_prefix(root)?.into(), fs::read(&path)?));
            }
        }
        Ok(())
    }
    let mut result = vec![];
    visit(root, root, &mut result)?;
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

#[tokio::test]
async fn preview_confirmation_restart_and_idempotency_preserve_evidence_without_replay()
-> Result<()> {
    timeout(Duration::from_secs(10), async {
        let f = Fixture::new()?;
        let (first, second) = f.interrupted().await?;
        let before = tree(f._dir.path())?;
        let preview = recover(first, &f.args(), &f.env)?;
        assert_eq!(tree(f._dir.path())?, before);
        assert!(preview.contains("Recovery preview") && preview.contains("partial system state"));
        assert!(
            !preview.contains(&"42".repeat(32)) && !preview.contains('—') && !preview.contains('–')
        );
        let args = f.confirm(&preview);
        let result = recover(first, &args, &f.env)?;
        assert!(result.contains("Activation was not rerun"));
        let receipt = super::super::read_receipt(&f.activation, first)?;
        assert_eq!(receipt.state, ActivationState::OperatorResolved);
        assert!(
            receipt
                .operator_resolution
                .as_ref()
                .unwrap()
                .no_writers_attested
        );
        let recorded = tree(f._dir.path())?;
        assert!(recover(first, &args, &f.env)?.contains("already recorded"));
        assert_eq!(tree(f._dir.path())?, recorded);
        let mut conflict = args.clone();
        conflict.reason = Some("different reason".into());
        assert!(recover(first, &conflict, &f.env).is_err());
        assert_eq!(tree(f._dir.path())?, recorded);
        let (jobs, mut started) = f.jobs().await?;
        assert_eq!(
            timeout(Duration::from_secs(4), started.recv())
                .await?
                .as_deref(),
            Some("next")
        );
        phase(&jobs, first, DeployPhase::Failed).await?;
        phase(&jobs, second, DeployPhase::Succeeded).await?;
        assert!(
            jobs.status(Some(first)).await?[0]
                .message
                .as_deref()
                .unwrap()
                .contains("operator attestation")
        );
        let third = jobs.accept(request("new")).await?;
        assert_eq!(started.recv().await.as_deref(), Some("new"));
        phase(&jobs, third, DeployPhase::Succeeded).await?;
        assert!(started.try_recv().is_err());
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn failed_preconditions_stale_bindings_and_busy_leases_never_write() -> Result<()> {
    timeout(Duration::from_secs(12), async {
        let f = Fixture::new()?;
        let (first, _) = f.interrupted().await?;
        let preview = recover(first, &f.args(), &f.env)?;
        let args = f.confirm(&preview);
        let before = tree(f._dir.path())?;
        let (jobs, _) = f.jobs().await?;
        assert!(
            recover(first, &args, &f.env)
                .unwrap_err()
                .to_string()
                .contains("owned")
        );
        jobs.shutdown().await;
        drop(jobs);
        assert_eq!(tree(f._dir.path())?, before);
        let store = ReceiptStore::existing(&f.activation)?;
        assert!(
            recover(first, &args, &f.env)
                .unwrap_err()
                .to_string()
                .contains("owned")
        );
        drop(store);
        assert_eq!(tree(f._dir.path())?, before);
        let mut no_attestation = args.clone();
        no_attestation.attest_no_writers = false;
        assert!(recover(first, &no_attestation, &f.env).is_err());
        let mut stale = args.clone();
        stale.confirm = Some("0".repeat(64));
        assert!(recover(first, &stale, &f.env).is_err());
        assert!(recover(999, &args, &f.env).is_err());
        assert_eq!(tree(f._dir.path())?, before);
        let store = ReceiptStore::existing(&f.activation)?;
        let original = store.load(first)?;
        let mut changed = original.clone();
        changed.message = "changed since preview".into();
        store.save(&changed)?;
        drop(store);
        let changed_tree = tree(f._dir.path())?;
        assert!(
            recover(first, &args, &f.env)
                .unwrap_err()
                .to_string()
                .contains("stale")
        );
        assert_eq!(tree(f._dir.path())?, changed_tree);
        let store = ReceiptStore::existing(&f.activation)?;
        changed.state = ActivationState::Ready;
        store.save(&changed)?;
        drop(store);
        let ready_tree = tree(f._dir.path())?;
        assert!(recover(first, &f.args(), &f.env).is_err());
        assert_eq!(tree(f._dir.path())?, ready_tree);
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn activating_is_eligible_but_malformed_audit_and_fsync_failure_fail_closed() -> Result<()> {
    timeout(Duration::from_secs(12), async {
        let f = Fixture::new()?;
        let (first, second) = f.interrupted().await?;
        let store = ReceiptStore::existing(&f.activation)?;
        let mut receipt = store.load(first)?;
        receipt.state = ActivationState::Activating;
        store.save(&receipt)?;
        drop(store);
        let args = f.confirm(&recover(first, &f.args(), &f.env)?);
        let fault = crate::activation::sync_fault::fail_next(&f.activation);
        assert!(recover(first, &args, &f.env).is_err());
        drop(fault);
        // Crash after receipt rename, before daemon ledger settlement: full audit is
        // available, but restart must re-save+fsync it before releasing FIFO.
        let resolved = super::super::read_receipt(&f.activation, first)?;
        assert_eq!(resolved.state, ActivationState::OperatorResolved);
        let receipt_path = f.activation.join(format!("{first}.toml"));
        let good = fs::read(&receipt_path)?;
        let mut malformed = resolved.clone();
        malformed
            .operator_resolution
            .as_mut()
            .unwrap()
            .no_writers_attested = false;
        fs::write(&receipt_path, toml::to_string(&malformed)?)?;
        let (jobs, mut started) = f.jobs().await?;
        assert!(
            timeout(Duration::from_millis(400), started.recv())
                .await
                .is_err()
        );
        assert_eq!(
            jobs.status(Some(first)).await?[0].phase,
            DeployPhase::Interrupted
        );
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        assert!(jobs.accept(request("blocked")).await.is_err());
        jobs.shutdown().await;
        drop(jobs);
        fs::write(&receipt_path, good)?;
        let (jobs, mut started) = f.jobs().await?;
        assert_eq!(
            timeout(Duration::from_secs(4), started.recv())
                .await?
                .as_deref(),
            Some("next")
        );
        phase(&jobs, first, DeployPhase::Failed).await?;
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[test]
fn missing_existing_state_and_uninitialized_ledger_are_never_created_or_migrated() -> Result<()> {
    let f = Fixture::new()?;
    let before = tree(f._dir.path())?;
    assert!(recover(1, &f.args(), &f.env).is_err());
    assert_eq!(tree(f._dir.path())?, before);
    fs::create_dir(&f.ledger)?;
    fs::set_permissions(&f.ledger, fs::Permissions::from_mode(0o700))?;
    fs::write(f.ledger.join("ledger.lock"), "")?;
    fs::set_permissions(
        f.ledger.join("ledger.lock"),
        fs::Permissions::from_mode(0o600),
    )?;
    let before = tree(f._dir.path())?;
    assert!(recover(1, &f.args(), &f.env).is_err());
    assert_eq!(tree(f._dir.path())?, before);
    Ok(())
}

struct NeverActivate;
impl ActivationExecutor for NeverActivate {
    fn current(&self) -> Result<PathBuf> {
        panic!("recovery must not query active system")
    }
    fn validate<'a>(&'a self, _: &'a Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { panic!("recovery must not validate activation") })
    }
    fn switch<'a>(&'a self, _: &'a Receipt, _: Duration) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { panic!("recovery must not replay activation") })
    }
    fn verify<'a>(&'a self, _: &'a Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { panic!("operator resolution must not become Verified") })
    }
}

#[tokio::test]
async fn resolved_receipt_never_reactivates_and_ledger_write_failure_blocks_same_run() -> Result<()>
{
    timeout(Duration::from_secs(10), async {
        let f = Fixture::new()?;
        let (first, second) = f.interrupted().await?;
        let args = f.confirm(&recover(first, &f.args(), &f.env)?);
        recover(first, &args, &f.env)?;
        let store = ReceiptStore::existing(&f.activation)?;
        assert!(
            super::super::activate(&store, first, &NeverActivate)
                .await
                .is_err()
        );
        assert!(
            super::super::reconcile(&store, first, &NeverActivate)
                .await
                .is_err()
        );
        // Hold activation lease across startup, then inject the actual ledger write
        // failure boundary before the observer can commit the recovered outcome.
        let (jobs, mut started) = f.jobs().await?;
        fs::create_dir(f.ledger.join("snapshot.pending"))?;
        drop(store);
        timeout(Duration::from_secs(4), async {
            loop {
                let message = jobs.status(Some(first)).await?[0]
                    .message
                    .clone()
                    .unwrap_or_default();
                if message.contains("persistence failure") {
                    break Ok::<_, eyre::Report>(());
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await??;
        assert!(
            timeout(Duration::from_millis(100), started.recv())
                .await
                .is_err()
                || started.is_closed()
        );
        assert!(jobs.accept(request("blocked")).await.is_err());
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Interrupted
        );
        jobs.shutdown().await;
        drop(jobs);
        fs::remove_dir(f.ledger.join("snapshot.pending"))?;
        let (jobs, mut started) = f.jobs().await?;
        assert_eq!(
            timeout(Duration::from_secs(4), started.recv())
                .await?
                .as_deref(),
            Some("next")
        );
        phase(&jobs, first, DeployPhase::Failed).await?;
        phase(&jobs, second, DeployPhase::Succeeded).await?;
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

// Exact v2 positional disk shape, used only to model a second interrupted attempt
// left by an earlier daemon. No production deserializer/type visibility changes.
#[derive(serde::Serialize, serde::Deserialize)]
struct CrashSnapshot {
    version: u8,
    scope: String,
    sequence: u64,
    request: DeployRequest,
    status: crate::common::proto::JobStatus,
    entries: std::collections::VecDeque<crate::common::proto::LogEntry>,
    cursor: u64,
    started: bool,
}

#[tokio::test]
async fn one_operator_resolution_cannot_release_another_uncertain_barrier() -> Result<()> {
    timeout(Duration::from_secs(12), async {
        let f = Fixture::new()?;
        let (jobs, mut started) = f.jobs().await?;
        let first = jobs.accept(request("partial")).await?;
        started.recv().await;
        let second = jobs.accept(request("second-partial")).await?;
        let third = jobs.accept(request("last")).await?;
        jobs.cancel(first).await?;
        phase(&jobs, first, DeployPhase::Interrupted).await?;
        jobs.shutdown().await;
        drop(jobs);
        let path = f.ledger.join(format!("{second}.job"));
        let mut crash: CrashSnapshot = postcard::from_bytes(&fs::read(&path)?)?;
        crash.status.phase = DeployPhase::Interrupted;
        crash.started = true;
        fs::write(path, postcard::to_stdvec(&crash)?)?;
        let store = ReceiptStore::existing(&f.activation)?;
        let mut receipt = store.load(first)?;
        receipt.job_id = second;
        receipt.root = store.root(second);
        symlink(&f.env.closure, &receipt.root)?;
        store.save(&receipt)?;
        drop(store);
        recover(
            first,
            &f.confirm(&recover(first, &f.args(), &f.env)?),
            &f.env,
        )?;
        let (jobs, mut started) = f.jobs().await?;
        phase(&jobs, first, DeployPhase::Failed).await?;
        assert!(
            timeout(Duration::from_millis(400), started.recv())
                .await
                .is_err()
        );
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Interrupted
        );
        assert_eq!(
            jobs.status(Some(third)).await?[0].phase,
            DeployPhase::Queued
        );
        assert!(jobs.accept(request("blocked")).await.is_err());
        jobs.shutdown().await;
        drop(jobs);
        recover(
            second,
            &f.confirm(&recover(second, &f.args(), &f.env)?),
            &f.env,
        )?;
        let (jobs, mut started) = f.jobs().await?;
        assert_eq!(
            timeout(Duration::from_secs(4), started.recv())
                .await?
                .as_deref(),
            Some("last")
        );
        phase(&jobs, second, DeployPhase::Failed).await?;
        phase(&jobs, third, DeployPhase::Succeeded).await?;
        assert!(started.try_recv().is_err());
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn challenge_binds_configuration_nonce_and_counter_and_rejects_unrelated_layout() -> Result<()>
{
    let f = Fixture::new()?;
    let (first, _) = f.interrupted().await?;
    let args = f.confirm(&recover(first, &f.args(), &f.env)?);
    let config = fs::read(&f.config)?;
    let alias = f._dir.path().join("other-config");
    fs::write(&alias, &config)?;
    fs::set_permissions(&alias, fs::Permissions::from_mode(0o600))?;
    let mut other = args.clone();
    other.server_config = alias;
    let before = tree(f._dir.path())?;
    assert!(recover(first, &other, &f.env).is_err());
    assert_eq!(tree(f._dir.path())?, before);
    let mut changed = config.clone();
    changed.extend_from_slice(b"\n# changed config generation\n");
    fs::write(&f.config, changed)?;
    let before = tree(f._dir.path())?;
    assert!(
        recover(first, &args, &f.env)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert_eq!(tree(f._dir.path())?, before);
    fs::write(&f.config, config)?;
    let job_path = f.ledger.join(format!("{first}.job"));
    let original = fs::read(&job_path)?;
    let mut crash: CrashSnapshot = postcard::from_bytes(&original)?;
    crash.request.request_id ^= 1 << 63;
    fs::write(&job_path, postcard::to_stdvec(&crash)?)?;
    let before = tree(f._dir.path())?;
    assert!(
        recover(first, &args, &f.env)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert_eq!(tree(f._dir.path())?, before);
    fs::write(&job_path, original)?;
    let counter = f.ledger.join("id-counter");
    let original = fs::read(&counter)?;
    fs::write(&counter, 10u64.to_be_bytes())?;
    let before = tree(f._dir.path())?;
    assert!(
        recover(first, &args, &f.env)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert_eq!(tree(f._dir.path())?, before);
    fs::write(counter, original)?;
    Ok(())
}

#[tokio::test]
async fn consistent_audit_for_wrong_nonce_stays_blocked_and_observer_remains_live() -> Result<()> {
    timeout(Duration::from_secs(10), async {
        let f = Fixture::new()?;
        let (first, second) = f.interrupted().await?;
        let args = f.confirm(&recover(first, &f.args(), &f.env)?);
        recover(first, &args, &f.env)?;
        let store = ReceiptStore::existing(&f.activation)?;
        let good = store.load(first)?;
        let mut wrong = good.clone();
        let audit = wrong.operator_resolution.as_mut().unwrap();
        audit.binding.request_nonce ^= 1 << 63;
        audit.challenge = challenge(&audit.prior_receipt_sha256, &audit.binding)?;
        store.save(&wrong)?;
        drop(store);
        let (jobs, mut started) = f.jobs().await?;
        assert!(
            timeout(Duration::from_millis(400), started.recv())
                .await
                .is_err()
        );
        assert_eq!(
            jobs.status(Some(first)).await?[0].phase,
            DeployPhase::Interrupted
        );
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        assert!(jobs.accept(request("blocked")).await.is_err());
        // Restoring the original immutable fixture audit proves the same observer
        // is alive; no daemon restart/replay is needed to observe a valid receipt.
        timeout(Duration::from_secs(3), async {
            loop {
                match ReceiptStore::existing(&f.activation) {
                    Ok(store) => {
                        store.save(&good)?;
                        return Ok::<_, eyre::Report>(());
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
                }
            }
        })
        .await??;
        assert_eq!(
            timeout(Duration::from_secs(4), started.recv())
                .await?
                .as_deref(),
            Some("next")
        );
        phase(&jobs, first, DeployPhase::Failed).await?;
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
