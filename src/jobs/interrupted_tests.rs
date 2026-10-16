//! Real FIFO/ledger/receipt recovery path with synthetic effects only.
use super::{
    handoff_tests::{blocked, directory, open, receipt},
    tests::request,
    *,
};
use crate::{
    activation::{ActivationState, ReceiptStore},
    common::proto::DeployPhase,
};
use eyre::Result;
use std::time::Duration;
use tokio::time::timeout;

async fn lease(path: &std::path::Path) -> Result<ReceiptStore> {
    timeout(Duration::from_secs(3), async {
        loop {
            match ReceiptStore::open(path) {
                Ok(store) => return Ok(store),
                Err(error) if error.to_string().contains("already owned") => {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await?
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
async fn terminal_receipt(path: &std::path::Path, id: u64, state: ActivationState) -> Result<()> {
    let store = lease(&directory(path)?).await?;
    let mut receipt = store.load(id)?;
    receipt.state = state;
    store.save(&receipt)
}

#[tokio::test]
async fn cancelled_activation_settles_then_resumes_queued_and_new_deploy_without_replay()
-> Result<()> {
    timeout(Duration::from_secs(10), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("state");
        let (jobs, mut started) = open(&path).await?;
        let original = request("uncertain-switch-fail");
        let first = jobs.accept(original.clone()).await?;
        started.recv().await;
        let second = jobs.accept(request("ordinary")).await?;
        jobs.cancel(first).await?;
        phase(&jobs, first, DeployPhase::Interrupted).await?;
        blocked(&mut started).await;
        assert!(jobs.accept(request("blocked")).await.is_err());
        assert_eq!(jobs.machine_status().await?.active.unwrap().job_id, first);
        // A locked terminal receipt is not yet observable; lease is mandatory.
        let store = lease(&directory(&path)?).await?;
        let mut observed = store.load(first)?;
        observed.state = ActivationState::Verified;
        store.save(&observed)?;
        blocked(&mut started).await;
        assert_eq!(
            jobs.status(Some(first)).await?[0].phase,
            DeployPhase::Interrupted
        );
        drop(store);
        phase(&jobs, first, DeployPhase::Succeeded).await?;
        assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
        phase(&jobs, second, DeployPhase::Succeeded).await?;
        assert_eq!(jobs.accept(original).await?, first);
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
async fn restart_observes_interrupted_receipt_keeps_queue_and_never_reexecutes_old_job()
-> Result<()> {
    timeout(Duration::from_secs(12), async {
        for state in [ActivationState::Verified, ActivationState::Cancelled] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("state");
            let (jobs, mut started) = open(&path).await?;
            let first = jobs.accept(request("manual")).await?;
            started.recv().await;
            let store = lease(&directory(&path)?).await?;
            let mut active = store.load(first)?;
            active.state = ActivationState::Uncertain;
            store.save(&active)?;
            drop(store);
            let second = jobs.accept(request("ordinary")).await?;
            jobs.cancel(first).await?;
            phase(&jobs, first, DeployPhase::Interrupted).await?;
            jobs.shutdown().await;
            drop(jobs);
            let (jobs, mut started) = open(&path).await?;
            assert_eq!(
                jobs.status(Some(second)).await?[0].phase,
                DeployPhase::Queued
            );
            blocked(&mut started).await;
            assert!(jobs.accept(request("blocked")).await.is_err());
            terminal_receipt(&path, first, state).await?;
            phase(
                &jobs,
                first,
                if state == ActivationState::Verified {
                    DeployPhase::Succeeded
                } else {
                    DeployPhase::Cancelled
                },
            )
            .await?;
            assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
            phase(&jobs, second, DeployPhase::Succeeded).await?;
            assert!(started.try_recv().is_err());
            jobs.shutdown().await;
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn every_interrupted_barrier_must_settle_before_queued_effects() -> Result<()> {
    timeout(Duration::from_secs(8), async {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("state");
        let (storage, _) = storage::JobStorage::open(path.clone(), "fixture".into()).await?;
        let activation = ReceiptStore::open(&directory(&path)?)?;
        for id in 1..=3 {
            let mut record = ledger::JobRecord::new(
                request(if id == 3 { "ordinary" } else { "never-replay" }),
                id,
                Default::default(),
            );
            record.status.job_id = id;
            if id < 3 {
                record.started = true;
                record.status.phase = DeployPhase::Interrupted;
                let mut receipt = receipt(&activation, id);
                receipt.state = if id == 1 {
                    ActivationState::Verified
                } else {
                    ActivationState::Uncertain
                };
                activation.save(&receipt)?;
            }
            storage.reserve_id(id).await?;
            storage.save(&record).await?;
        }
        drop(activation);
        drop(storage);
        let (jobs, mut started) = open(&path).await?;
        phase(&jobs, 1, DeployPhase::Succeeded).await?;
        blocked(&mut started).await;
        assert!(jobs.accept(request("blocked")).await.is_err());
        assert_eq!(jobs.machine_status().await?.active.unwrap().job_id, 2);
        terminal_receipt(&path, 2, ActivationState::Verified).await?;
        assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
        phase(&jobs, 3, DeployPhase::Succeeded).await?;
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn missing_or_corrupt_interrupted_receipt_never_releases_fifo() -> Result<()> {
    timeout(Duration::from_secs(10), async {
        for corrupt in [false, true] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("state");
            let (jobs, mut started) = open(&path).await?;
            let first = jobs.accept(request("uncertain-switch-fail")).await?;
            started.recv().await;
            let second = jobs.accept(request("ordinary")).await?;
            jobs.cancel(first).await?;
            phase(&jobs, first, DeployPhase::Interrupted).await?;
            let receipt_path = directory(&path)?.join(format!("{first}.toml"));
            let original = std::fs::read(&receipt_path)?;
            if corrupt {
                std::fs::write(&receipt_path, "invalid = [")?;
            } else {
                std::fs::remove_file(&receipt_path)?;
            }
            blocked(&mut started).await;
            assert_eq!(
                jobs.status(Some(second)).await?[0].phase,
                DeployPhase::Queued
            );
            assert!(jobs.accept(request("blocked")).await.is_err());
            jobs.shutdown().await;
            drop(jobs);
            let (jobs, mut started) = open(&path).await?;
            blocked(&mut started).await;
            assert!(jobs.accept(request("blocked")).await.is_err());
            std::fs::write(&receipt_path, original)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&receipt_path, std::fs::Permissions::from_mode(0o600))?;
            // Restoring uncertainty is not settlement either.
            blocked(&mut started).await;
            assert_eq!(
                jobs.status(Some(first)).await?[0].phase,
                DeployPhase::Interrupted
            );
            terminal_receipt(&path, first, ActivationState::Verified).await?;
            assert_eq!(started.recv().await.as_deref(), Some("ordinary"));
            phase(&jobs, second, DeployPhase::Succeeded).await?;
            jobs.shutdown().await;
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
