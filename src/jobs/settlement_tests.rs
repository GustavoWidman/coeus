//! Post-preparation cancellation and durability regressions; no host effects.
use super::{
    handoff_tests::{blocked, directory, open, receipt},
    tests::{request, terminal},
    *,
};
use crate::{
    activation::{ActivationState, ReceiptStore},
    common::proto::DeployPhase,
};
use eyre::Result;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::timeout};

/// Pause exactly after Ready preparation while the ledger is still Build.
struct PreparedExecutor {
    started: mpsc::UnboundedSender<String>,
    release: Arc<tokio::sync::Notify>,
}
impl DeploymentExecutor for PreparedExecutor {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if request.rev == "ordinary" {
                self.started.send(request.rev)?;
                return Ok(());
            }
            context.phase(DeployPhase::Build).await?;
            let store = ReceiptStore::open(&context.activation_directory().await?)?;
            store.save(&receipt(&store, context.job_id()))?;
            drop(store);
            self.started.send(request.rev.clone())?;
            self.release.notified().await;
            match request.rev.as_str() {
                "before-switch" => context.phase(DeployPhase::Switch).await?,
                "before-handoff" => context.handoff().await?,
                "dry-run-revoke" => context.revoke_build_only().await?,
                _ => {
                    eyre::bail!("fixture failed in Build after Ready preparation");
                }
            }
            Ok(())
        })
    }
}
async fn prepared_jobs(
    path: &Path,
) -> Result<(
    DeploymentJobs,
    mpsc::UnboundedReceiver<String>,
    Arc<tokio::sync::Notify>,
)> {
    let (send, recv) = mpsc::unbounded_channel();
    let release = Arc::new(tokio::sync::Notify::new());
    let jobs = DeploymentJobs::open(
        Arc::new(PreparedExecutor {
            started: send,
            release: release.clone(),
        }),
        path.into(),
        "fixture".into(),
    )
    .await?;
    Ok((jobs, recv, release))
}

#[tokio::test]
async fn cancellation_after_ready_before_switch_or_handoff_revokes_in_build() -> Result<()> {
    timeout(Duration::from_secs(8), async {
        for rev in ["before-switch", "before-handoff"] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("ledger");
            let (jobs, mut started, release) = prepared_jobs(&path).await?;
            let original = request(rev);
            let first = jobs.accept(original.clone()).await?;
            assert_eq!(started.recv().await.as_deref(), Some(rev));
            assert_eq!(jobs.status(Some(first)).await?[0].phase, DeployPhase::Build);
            let second = jobs.accept(request("ordinary")).await?;
            jobs.cancel(first).await?;
            release.notify_one();
            assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
            assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Cancelled);
            assert_eq!(
                crate::activation::read_receipt(&directory(&path)?, first)?.state,
                ActivationState::Cancelled
            );
            assert_eq!(jobs.accept(original).await?, first);
            terminal(&jobs, second).await;
            jobs.shutdown().await;
            drop(jobs);
            let (recovered, mut restarted, _) = prepared_jobs(&path).await?;
            assert_eq!(
                recovered.status(Some(first)).await?[0].phase,
                DeployPhase::Cancelled
            );
            blocked(&mut restarted).await; // no build/activation replay
            recovered.shutdown().await;
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

async fn assert_persistence_stop(
    jobs: &DeploymentJobs,
    first: u64,
    second: u64,
    started: &mut mpsc::UnboundedReceiver<String>,
    path: &Path,
) -> Result<()> {
    let status = terminal(jobs, first).await;
    assert_eq!(status.phase, DeployPhase::Interrupted);
    assert!(
        status
            .message
            .unwrap()
            .contains("post-rename receipt directory fsync failure")
    );
    // Rename did happen. Its readable Cancelled state is explicitly NOT proof
    // that this failed persistence operation may release the FIFO.
    assert_eq!(
        crate::activation::read_receipt(&directory(path)?, first)?.state,
        ActivationState::Cancelled
    );
    assert_eq!(
        jobs.status(Some(second)).await?[0].phase,
        DeployPhase::Queued
    );
    assert!(jobs.accept(request("after-fsync-failure")).await.is_err());
    assert!(matches!(
        timeout(Duration::from_millis(50), started.recv()).await,
        Err(_) | Ok(None)
    ));
    Ok(())
}

async fn assert_restart_stays_blocked(path: &Path, first: u64) -> Result<()> {
    let (jobs, mut started, _) = prepared_jobs(path).await?;
    assert_eq!(
        jobs.status(Some(first)).await?[0].phase,
        DeployPhase::Interrupted
    );
    assert!(jobs.accept(request("restart-attempt")).await.is_err());
    blocked(&mut started).await;
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn post_rename_fsync_failure_in_build_revocation_and_dry_run_stops_admission() -> Result<()> {
    timeout(Duration::from_secs(8), async {
        for rev in ["before-switch", "dry-run-revoke"] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("ledger");
            let (jobs, mut started, release) = prepared_jobs(&path).await?;
            let first = jobs.accept(request(rev)).await?;
            started.recv().await;
            let second = jobs.accept(request("ordinary")).await?;
            let _fault = crate::activation::sync_fault::fail_next(&directory(&path)?);
            if rev == "before-switch" {
                jobs.cancel(first).await?;
            }
            release.notify_one();
            assert_persistence_stop(&jobs, first, second, &mut started, &path).await?;
            jobs.shutdown().await;
            drop(jobs);
            assert_restart_stays_blocked(&path, first).await?;
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn post_rename_fsync_failure_in_handoff_cancellation_stops_admission() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("manual")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        // Own the lease while arming cancellation/fault so the test cannot race
        // an earlier observer poll. The real observer revokes after release.
        let operator = ReceiptStore::open(&directory(&path)?)?;
        jobs.cancel(first).await?;
        let _fault = crate::activation::sync_fault::fail_next(&directory(&path)?);
        drop(operator);
        assert_persistence_stop(&jobs, first, second, &mut started, &path).await?;
        jobs.shutdown().await;
        drop(jobs);
        assert_restart_stays_blocked(&path, first).await?;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn build_phase_ready_failure_with_busy_lease_blocks_fifo_across_restart() -> Result<()> {
    timeout(Duration::from_secs(6), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started, release) = prepared_jobs(&path).await?;
        let first = jobs.accept(request("build-failure")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        let operator = ReceiptStore::open(&directory(&path)?)?;
        release.notify_one();
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
        assert_eq!(operator.load(first)?.state, ActivationState::Ready);
        blocked(&mut started).await;
        jobs.shutdown().await;
        drop(jobs);
        let (recovered, mut restarted, _) = prepared_jobs(&path).await?;
        jobs_cancel_and_check(&recovered, first, &operator).await?;
        blocked(&mut restarted).await;
        drop(operator);
        assert_eq!(restarted.recv().await.as_deref(), Some("ordinary"));
        assert_eq!(
            terminal(&recovered, first).await.phase,
            DeployPhase::Cancelled
        );
        terminal(&recovered, second).await;
        recovered.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
async fn jobs_cancel_and_check(
    jobs: &DeploymentJobs,
    id: u64,
    operator: &ReceiptStore,
) -> Result<()> {
    jobs.cancel(id).await?;
    assert_eq!(operator.load(id)?.state, ActivationState::Ready);
    Ok(())
}

#[tokio::test]
async fn readable_cancelled_receipt_cannot_release_fifo_while_operator_owns_lease() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("ledger");
        let (jobs, mut started) = open(&path).await?;
        let first = jobs.accept(request("manual")).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        let operator = ReceiptStore::open(&directory(&path)?)?;
        jobs.cancel(first).await?;
        let mut retained = operator.load(first)?;
        retained.state = ActivationState::Cancelled;
        operator.save(&retained)?;
        // Hold through more than two observer polls. A short FIFO check could
        // pass simply because the old unlocked observer had not run yet.
        assert!(
            timeout(Duration::from_millis(600), started.recv())
                .await
                .is_err()
        );
        assert_eq!(
            jobs.status(Some(first)).await?[0].phase,
            DeployPhase::RequiresManualSwitch
        );
        drop(operator);
        assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
        assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Cancelled);
        terminal(&jobs, second).await;
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
