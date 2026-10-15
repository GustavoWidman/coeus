//! Durable FIFO handoff through the real job worker and receipt observer, fake host effects only.
use super::tests::{request, terminal};
use super::*;
use crate::{
    activation::{ActivationState, Receipt, ReceiptStore, ServiceIdentity},
    common::proto::DeployPhase,
};
use eyre::Result;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};

struct HandoffExecutor {
    started: mpsc::UnboundedSender<String>,
}
fn service() -> ServiceIdentity {
    ServiceIdentity {
        host: "fixture".into(),
        platform: "linux".into(),
        daemon: "/nix/store/00000000000000000000000000000000-coeus/bin/coeusd".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        protocol: crate::common::proto::PROTOCOL_VERSION,
        user: "fixture".into(),
        account: crate::activation::AccountPolicy {
            managed: false,
            uid: None,
            gid: None,
        },
        state_directory: "/fixture/state".into(),
        config_file: "/fixture/server.toml".into(),
        runner: "/nix/store/00000000000000000000000000000000-runner".into(),
        elevation_adapter: "/nix/store/00000000000000000000000000000000-adapter".into(),
        nix: "/nix/store/00000000000000000000000000000000-nix/bin/nix".into(),
        service_definition: "fixture".into(),
    }
}
pub(super) fn receipt(store: &ReceiptStore, id: u64) -> Receipt {
    Receipt {
        format: 1,
        job_id: id,
        closure: "/nix/store/00000000000000000000000000000000-system".into(),
        root: store.root(id),
        expected: service(),
        previous: Some(service()),
        state: ActivationState::Ready,
        manual: true,
        previous_process_id: Some(123),
        message: "fake built closure".into(),
    }
}
impl DeploymentExecutor for HandoffExecutor {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.phase(DeployPhase::Build).await?;
            if request.rev.ends_with("switch-fail") {
                context.phase(DeployPhase::Switch).await?;
                if request.rev != "missing-switch-fail" {
                    let store = ReceiptStore::open(&context.activation_directory().await?)?;
                    let mut built = receipt(&store, context.job_id());
                    built.manual = false;
                    built.state = match request.rev.as_str() {
                        "uncertain-switch-fail" => ActivationState::Uncertain,
                        "activating-switch-fail" => ActivationState::Activating,
                        "verified-switch-fail" => ActivationState::Verified,
                        _ => ActivationState::Ready,
                    };
                    store.save(&built)?;
                    if request.rev == "revocation-switch-fail" {
                        std::fs::create_dir(store.directory().join("receipt.pending"))?;
                    }
                }
                self.started.send(request.rev)?;
                eyre::bail!("fixture activation denied: \x1b[31mpermission refused\x1b[0m");
            }
            if matches!(request.rev.as_str(), "manual" | "corrupt-switch") {
                let store = ReceiptStore::open(&context.activation_directory().await?)?;
                store.save(&receipt(&store, context.job_id()))?;
                if request.rev == "corrupt-switch" {
                    context.phase(DeployPhase::Switch).await?;
                    std::fs::write(
                        store.directory().join(format!("{}.toml", context.job_id())),
                        "invalid = [",
                    )?;
                    self.started.send(request.rev)?;
                    eyre::bail!("ambiguous activation with unreadable receipt");
                }
                drop(store);
                context.phase(DeployPhase::RequiresManualSwitch).await?;
                self.started.send(request.rev)?;
                context.handoff().await
            } else {
                self.started.send(request.rev)?;
                Ok(())
            }
        })
    }
}
pub(super) async fn open(path: &Path) -> Result<(DeploymentJobs, mpsc::UnboundedReceiver<String>)> {
    let (send, recv) = mpsc::unbounded_channel();
    Ok((
        DeploymentJobs::open(
            Arc::new(HandoffExecutor { started: send }),
            path.into(),
            "fixture".into(),
        )
        .await?,
        recv,
    ))
}
pub(super) fn directory(path: &Path) -> Result<PathBuf> {
    crate::activation::directory_for_ledger(path)
}
fn set_receipt(path: &Path, id: u64, state: ActivationState) -> Result<()> {
    let store = ReceiptStore::open(&directory(path)?)?;
    let mut receipt = store.load(id)?;
    receipt.state = state;
    store.save(&receipt)
}
pub(super) async fn blocked(recv: &mut mpsc::UnboundedReceiver<String>) {
    assert!(
        !matches!(
            timeout(Duration::from_millis(50), recv.recv()).await,
            Ok(Some(_))
        ),
        "FIFO crossed unresolved manual barrier"
    );
}
#[tokio::test]
async fn manual_switch_pauses_fifo_until_independent_verification() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let original = request("manual");
        let first = jobs.accept(original.clone()).await?;
        assert_eq!(started.recv().await.as_deref(), Some("manual"));
        let second = jobs.accept(request("ordinary")).await?;
        assert_eq!(jobs.accept(original).await?, first); // retries preserve the same barrier
        blocked(&mut started).await;
        set_receipt(&path, first, ActivationState::Verified)?;
        assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
        assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Succeeded);
        assert_eq!(terminal(&jobs, second).await.phase, DeployPhase::Succeeded);
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
#[tokio::test]
async fn replacement_restart_observes_handoff_and_resumes_only_never_started_fifo() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("manual")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        jobs.shutdown().await;
        drop(jobs);
        let (recovered, mut after_restart) = open(&path).await?;
        assert_eq!(
            recovered.status(Some(first)).await?[0].phase,
            DeployPhase::RequiresManualSwitch
        );
        assert_eq!(
            recovered.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        blocked(&mut after_restart).await; // no build/activation of handoff is replayed
        set_receipt(&path, first, ActivationState::Verified)?;
        assert_eq!(after_restart.recv().await.as_deref(), Some("ordinary"));
        terminal(&recovered, second).await;
        recovered.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
#[tokio::test]
async fn ready_cancellation_unblocks_but_inflight_cancellation_settles_uncertain() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        // Safe, never-started activation is durably revoked and FIFO can proceed.
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("manual")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        jobs.cancel(first).await?;
        assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Cancelled);
        assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
        terminal(&jobs, second).await;
        assert_eq!(
            crate::activation::read_receipt(&directory(&path)?, first)?.state,
            ActivationState::Cancelled
        );
        jobs.shutdown().await;

        // In-flight cancellation settles boundedly without claiming rollback.
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("manual")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        set_receipt(&path, first, ActivationState::Activating)?;
        jobs.cancel(first).await?;
        let status = terminal(&jobs, first).await;
        assert_eq!(status.phase, DeployPhase::Interrupted);
        assert!(status.cancel_requested);
        assert!(status.message.unwrap().contains("outcome uncertain"));
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        blocked(&mut started).await;
        assert!(jobs.accept(request("blocked")).await.is_err());
        jobs.cancel(second).await?;
        assert_eq!(terminal(&jobs, second).await.phase, DeployPhase::Cancelled);
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
#[tokio::test]
async fn unresolved_or_corrupt_handoff_never_runs_queued_job_even_after_shutdown() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("manual")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        set_receipt(&path, first, ActivationState::Uncertain)?;
        blocked(&mut started).await;
        jobs.shutdown().await;
        assert_eq!(
            jobs.status(Some(first)).await?[0].phase,
            DeployPhase::RequiresManualSwitch
        );
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn verified_receipt_does_not_release_fifo_until_operator_lease_is_released() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("manual")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        let operator = ReceiptStore::open(&directory(&path)?)?;
        let mut receipt = operator.load(first)?;
        receipt.state = ActivationState::Verified;
        operator.save(&receipt)?;
        assert!(
            timeout(Duration::from_millis(600), started.recv())
                .await
                .is_err(),
            "FIFO crossed while operator still owns lease"
        );
        drop(operator);
        assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
        terminal(&jobs, second).await;
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn cancelled_inflight_handoff_survives_restart_without_replay() -> Result<()> {
    timeout(Duration::from_secs(8), async {
        for state in [ActivationState::Activating, ActivationState::Uncertain] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("ledger");
            let (jobs, mut started) = open(&path).await?;
            let first = jobs.accept(request("manual")).await?;
            started.recv().await;
            let second = jobs.accept(request("ordinary")).await?;
            set_receipt(&path, first, state)?;
            jobs.cancel(first).await?;
            assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Interrupted);
            jobs.shutdown().await;
            drop(jobs);
            let (recovered, mut after_restart) = open(&path).await?;
            let status = &recovered.status(Some(first)).await?[0];
            assert!(status.cancel_requested);
            assert_eq!(status.phase, DeployPhase::Interrupted);
            assert_eq!(
                recovered.status(Some(second)).await?[0].phase,
                DeployPhase::Interrupted
            );
            assert_eq!(
                crate::activation::read_receipt(&directory(&path)?, first)?.state,
                state
            );
            blocked(&mut after_restart).await;
            assert!(recovered.accept(request("blocked")).await.is_err());
            recovered.shutdown().await;
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
#[tokio::test]
async fn leased_receipt_cancellation_cannot_claim_safe_revocation_or_replay() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("manual")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        let operator = ReceiptStore::open(&directory(&path)?)?;
        jobs.cancel(first).await?;
        assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Interrupted);
        assert_eq!(operator.load(first)?.state, ActivationState::Ready);
        jobs.shutdown().await;
        drop(jobs);
        drop(operator);
        let (recovered, mut after_restart) = open(&path).await?;
        assert_eq!(
            recovered.status(Some(first)).await?[0].phase,
            DeployPhase::Interrupted
        );
        assert_eq!(
            recovered.status(Some(second)).await?[0].phase,
            DeployPhase::Interrupted
        );
        blocked(&mut after_restart).await;
        assert_eq!(
            crate::activation::read_receipt(&directory(&path)?, first)?.state,
            ActivationState::Ready
        );
        recovered.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
#[tokio::test]
async fn unreadable_switch_outcome_blocks_fifo_across_restart_without_replay() -> Result<()> {
    timeout(Duration::from_secs(6), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("corrupt-switch")).await?;
        assert_eq!(started.recv().await.as_deref(), Some("corrupt-switch"));
        let second = jobs.accept(request("ordinary")).await?;
        timeout(Duration::from_secs(2), async {
            loop {
                if jobs.status(Some(first)).await?[0].phase == DeployPhase::RequiresManualSwitch {
                    break;
                }
                tokio::task::yield_now().await;
            }
            Ok::<_, eyre::Report>(())
        })
        .await??;
        blocked(&mut started).await;
        jobs.shutdown().await;
        drop(jobs);
        let (recovered, mut after_restart) = open(&path).await?;
        blocked(&mut after_restart).await; // cannot activate or cross corrupt receipt
        assert_eq!(
            recovered.status(Some(first)).await?[0].phase,
            DeployPhase::RequiresManualSwitch
        );
        assert_eq!(
            recovered.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        // An independent reconciler restores verified evidence, not a replayed executor.
        let store = ReceiptStore::open(&directory(&path)?)?;
        let mut restored = receipt(&store, first);
        restored.state = ActivationState::Verified;
        store.save(&restored)?;
        drop(store);
        assert_eq!(after_restart.recv().await.as_deref(), Some("ordinary"));
        assert_eq!(
            terminal(&recovered, first).await.phase,
            DeployPhase::Succeeded
        );
        terminal(&recovered, second).await;
        recovered.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn pre_activation_failure_revokes_ready_receipt_and_preserves_cause() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let original = request("ready-switch-fail");
        let first = jobs.accept(original.clone()).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
        let status = terminal(&jobs, first).await;
        assert_eq!(status.phase, DeployPhase::Failed);
        let message = status.message.unwrap();
        assert!(message.contains("fixture activation denied: permission refused"));
        assert!(!message.chars().any(char::is_control));
        let retained = crate::activation::read_receipt(&directory(&path)?, first)?;
        assert_eq!(retained.state, ActivationState::Cancelled);
        assert!(retained.message.contains("failed before activation intent"));
        assert!(retained.message.contains("permission refused"));
        assert_eq!(jobs.accept(original).await?, first); // no retry/rebuild
        assert!(jobs.logs(first, 0).await?.entries.iter().any(|e| {
            e.text
                .contains("activation failed: fixture activation denied: permission refused")
        }));
        terminal(&jobs, second).await;
        jobs.shutdown().await;
        drop(jobs);
        let (recovered, mut after_restart) = open(&path).await?;
        assert_eq!(
            recovered.status(Some(first)).await?[0].phase,
            DeployPhase::Failed
        );
        blocked(&mut after_restart).await;
        recovered.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn attempted_or_missing_activation_keeps_cause_and_blocks_until_verified() -> Result<()> {
    timeout(Duration::from_secs(8), async {
        for rev in [
            "uncertain-switch-fail",
            "activating-switch-fail",
            "missing-switch-fail",
        ] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("ledger");
            let (jobs, mut started) = open(&path).await?;
            let first = jobs.accept(request(rev)).await?;
            started.recv().await;
            let second = jobs.accept(request("ordinary")).await?;
            timeout(Duration::from_secs(2), async {
                loop {
                    let status = jobs.status(Some(first)).await?.remove(0);
                    if status.phase == DeployPhase::RequiresManualSwitch {
                        assert!(status.message.unwrap().contains("permission refused"));
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                Ok::<_, eyre::Report>(())
            })
            .await??;
            blocked(&mut started).await;
            assert!(jobs.logs(first, 0).await?.entries.iter().any(|e| {
                e.text
                    .contains("activation failed: fixture activation denied: permission refused")
            }));
            jobs.shutdown().await;
            assert!(
                jobs.status(Some(first)).await?[0]
                    .message
                    .as_ref()
                    .unwrap()
                    .contains("permission refused")
            );
            drop(jobs);
            let (recovered, mut after_restart) = open(&path).await?;
            blocked(&mut after_restart).await;
            let store = ReceiptStore::open(&directory(&path)?)?;
            let mut restored = receipt(&store, first);
            restored.state = ActivationState::Verified;
            store.save(&restored)?;
            drop(store);
            assert_eq!(after_restart.recv().await.as_deref(), Some("ordinary"));
            assert_eq!(
                terminal(&recovered, first).await.phase,
                DeployPhase::Succeeded
            );
            terminal(&recovered, second).await;
            recovered.shutdown().await;
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn failed_ready_revocation_never_releases_fifo_or_allows_new_admission() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("revocation-switch-fail")).await?;
        started.recv().await;
        let status = terminal(&jobs, first).await;
        assert_eq!(status.phase, DeployPhase::Interrupted);
        assert!(
            status
                .message
                .unwrap()
                .contains("revocation persistence failed")
        );
        assert!(jobs.accept(request("ordinary")).await.is_err());
        assert!(matches!(
            timeout(Duration::from_millis(50), started.recv()).await,
            Err(_) | Ok(None)
        ));
        jobs.shutdown().await;
        drop(jobs);
        let (recovered, mut after_restart) = open(&path).await?;
        assert!(recovered.accept(request("ordinary")).await.is_err());
        blocked(&mut after_restart).await;
        recovered.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn verified_switch_error_is_not_reported_as_failed_or_replayed() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("ledger");
    let (jobs, mut started) = open(&path).await?;
    let first = jobs.accept(request("verified-switch-fail")).await?;
    started.recv().await;
    assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Succeeded);
    assert!(
        jobs.logs(first, 0)
            .await?
            .entries
            .iter()
            .any(|e| e.text.contains("permission refused"))
    );
    jobs.shutdown().await;
    Ok(())
}

struct LeasedFailureExecutor {
    lease: Arc<std::sync::Mutex<Option<ReceiptStore>>>,
    started: mpsc::UnboundedSender<()>,
}
impl DeploymentExecutor for LeasedFailureExecutor {
    fn execute(&self, _: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let store = ReceiptStore::open(&context.activation_directory().await?)?;
            store.save(&receipt(&store, context.job_id()))?;
            context.phase(DeployPhase::Switch).await?;
            *self.lease.lock().unwrap() = Some(store);
            self.started.send(())?;
            eyre::bail!("fixture lease held by independent operator");
        })
    }
}
#[tokio::test]
async fn ready_receipt_with_busy_operator_lease_is_not_revoked_or_treated_as_safe() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let lease = Arc::new(std::sync::Mutex::new(None));
        let (send, mut recv) = mpsc::unbounded_channel();
        let jobs = DeploymentJobs::open(
            Arc::new(LeasedFailureExecutor {
                lease: lease.clone(),
                started: send,
            }),
            path.clone(),
            "fixture".into(),
        )
        .await?;
        let first = jobs.accept(request("busy-ready")).await?;
        recv.recv().await;
        timeout(Duration::from_secs(2), async {
            loop {
                if jobs.status(Some(first)).await?[0].phase == DeployPhase::RequiresManualSwitch {
                    break;
                }
                tokio::task::yield_now().await;
            }
            Ok::<_, eyre::Report>(())
        })
        .await??;
        assert_eq!(
            lease.lock().unwrap().as_ref().unwrap().load(first)?.state,
            ActivationState::Ready
        );
        assert!(jobs.logs(first, 0).await?.entries.iter().any(|e| {
            e.text
                .contains("activation is already owned by another process")
        }));
        jobs.cancel(first).await?;
        assert!(
            timeout(Duration::from_millis(50), recv.recv())
                .await
                .is_err()
        );
        // Cancel can only revoke Ready after the independent lease is released.
        lease.lock().unwrap().take();
        assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Cancelled);
        assert_eq!(
            crate::activation::read_receipt(&directory(&path)?, first)?.state,
            ActivationState::Cancelled
        );
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
