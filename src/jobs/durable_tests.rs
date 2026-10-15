//! Throwaway private ledger fixtures only; no Deployer/nh/production configs.
use super::ledger::JobRecord;
use super::tests::{QuickExecutor, request};
use super::*;
use crate::common::proto::{DeployPhase, JobId, JobStatus, LogSource};
use eyre::Result;
use std::{path::Path, sync::Arc};

async fn open(path: &Path) -> Result<DeploymentJobs> {
    DeploymentJobs::open(
        Arc::new(QuickExecutor),
        path.to_path_buf(),
        "fixture".into(),
    )
    .await
}

#[tokio::test]
async fn terminal_failure_cause_survives_restart_in_status_and_logs() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let jobs = open(&state).await?;
    let original = request("fail");
    let id = jobs.accept(original.clone()).await?;
    assert_eq!(terminal(&jobs, id).await.phase, DeployPhase::Failed);
    jobs.shutdown().await;
    drop(jobs);
    let recovered = open(&state).await?;
    assert_eq!(recovered.accept(original).await?, id);
    let status = &recovered.status(Some(id)).await?[0];
    assert_eq!(status.phase, DeployPhase::Failed);
    assert_eq!(status.message.as_deref(), Some("isolated failure"));
    let logs = recovered.logs(id, 0).await?;
    assert!(logs.finished);
    assert_eq!(
        logs.entries.last().unwrap().text,
        "Failed — isolated failure"
    );
    recovered.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn durable_admission_deduplicates_and_conflicts_across_restart() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let jobs = open(&state).await?;
    let original = request("ok");
    let id = jobs.accept(original.clone()).await?;
    assert_eq!(jobs.accept(original.clone()).await?, id);
    assert!(state.join(format!("{id}.job")).exists());
    let mut conflict = original.clone();
    conflict.clean_substituters = true;
    assert!(jobs.accept(conflict.clone()).await.is_err());
    terminal(&jobs, id).await;
    let logs = jobs.logs(id, 0).await?;
    jobs.shutdown().await;
    drop(jobs);
    let recovered = open(&state).await?;
    assert_eq!(recovered.accept(original).await?, id);
    assert!(recovered.accept(conflict).await.is_err());
    assert_eq!(recovered.status(None).await?.len(), 1);
    assert_eq!(
        recovered.status(Some(id)).await?[0].phase,
        DeployPhase::Succeeded
    );
    assert_eq!(recovered.logs(id, 0).await?.next_cursor, logs.next_cursor);
    recovered.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn restart_interrupts_each_uncertain_phase_without_execution_or_admission() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let (storage, _) = storage::JobStorage::open(state.clone(), "fixture".into()).await?;
    let mut originals = Vec::new();
    for (index, phase) in [
        DeployPhase::Queued,
        DeployPhase::Checkout,
        DeployPhase::Build,
        DeployPhase::Switch,
    ]
    .into_iter()
    .enumerate()
    {
        let mut original = request("ok");
        original.request_id = index as u64 + 1;
        storage.reserve_id(original.request_id).await?;
        let mut record =
            JobRecord::new(original.clone(), index as u64 + 1, CancellationToken::new());
        record.status.phase = phase;
        record.append(LogSource::Stdout, "persisted output before simulated crash");
        storage.save(&record).await?;
        originals.push(original);
    }
    drop(storage);
    let jobs = open(&state).await?;
    for original in originals {
        let id = original.request_id;
        assert_eq!(jobs.accept(original).await?, id);
        assert_eq!(
            jobs.status(Some(id)).await?[0].phase,
            DeployPhase::Interrupted
        );
        let logs = jobs.logs(id, 0).await?;
        assert_eq!(
            logs.entries[0].text,
            "persisted output before simulated crash"
        );
        assert!(logs.finished);
    }
    assert!(
        jobs.accept(request("new"))
            .await
            .unwrap_err()
            .to_string()
            .contains("reconciliation")
    );
    jobs.shutdown().await;
    drop(jobs);
    let again = open(&state).await?;
    assert!(
        again
            .status(None)
            .await?
            .iter()
            .all(|job| job.phase == DeployPhase::Interrupted)
    );
    again.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn durable_retention_stops_admission_without_expiring_idempotency() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let jobs = open(&state).await?;
    let first = request("ok");
    let first_id = jobs.accept(first.clone()).await?;
    terminal(&jobs, first_id).await;
    for _ in 1..ledger::MAX_JOBS {
        let id = jobs.accept(request("ok")).await?;
        terminal(&jobs, id).await;
    }
    let retained = jobs.terminals.lock().await.clone();
    assert_eq!(retained.len(), ledger::MAX_JOBS);
    retained[&first_id]
        .lock()
        .unwrap()
        .push(b"durable history")?;
    assert!(jobs.accept(request("overflow")).await.is_err());
    assert_eq!(jobs.accept(first.clone()).await?, first_id);
    {
        let terminals = jobs.terminals.lock().await;
        assert_eq!(terminals.len(), ledger::MAX_JOBS);
        for (id, output) in &retained {
            assert!(Arc::ptr_eq(output, &terminals[id]));
        }
    }
    let page = jobs
        .terminal(crate::common::proto::TerminalRequest {
            job_id: first_id,
            after: Some(0),
            size: Default::default(),
        })
        .await?;
    assert!(page.available && page.finished && !page.gap);
    assert_eq!(page.bytes, b"durable history");
    jobs.shutdown().await;
    drop(jobs);
    let recovered = open(&state).await?;
    assert_eq!(recovered.accept(first).await?, first_id);
    assert_eq!(recovered.status(None).await?.len(), ledger::MAX_JOBS);
    assert!(std::fs::read_dir(&state)?.count() <= ledger::MAX_JOBS + 3);
    for entry in std::fs::read_dir(&state)? {
        assert!(entry?.metadata()?.len() <= 300 * 1024);
    }
    recovered.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn output_cursor_and_tail_survive_restart_with_bounded_snapshot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let jobs = open(&state).await?;
    let id = jobs.accept(request("logs")).await?;
    terminal(&jobs, id).await;
    let before = jobs.logs(id, 0).await?;
    assert!(before.gap && before.finished);
    jobs.shutdown().await;
    drop(jobs);
    let recovered = open(&state).await?;
    let after = recovered.logs(id, 0).await?;
    assert!(after.gap && after.finished);
    assert_eq!(after.next_cursor, before.next_cursor);
    assert_eq!(after.entries[0].text, before.entries[0].text);
    assert!(std::fs::metadata(state.join(format!("{id}.job")))?.len() <= 300 * 1024);
    recovered.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn single_owner_corrupt_snapshot_and_repository_scope_fail_closed() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let jobs = open(&state).await?;
    assert!(open(&state).await.is_err());
    let id = jobs.accept(request("ok")).await?;
    terminal(&jobs, id).await;
    jobs.shutdown().await;
    drop(jobs);
    assert!(
        DeploymentJobs::open(
            Arc::new(QuickExecutor),
            state.clone(),
            "other-repository".into()
        )
        .await
        .is_err()
    );
    std::fs::write(state.join(format!("{id}.job")), b"corrupt")?;
    assert!(open(&state).await.is_err());
    Ok(())
}

#[tokio::test]
async fn persistence_failure_stops_execution_and_never_returns_acceptance() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let jobs = open(&state).await?;
    // Deterministic rename failure, not a flaky disk-space test.
    let original = request("ok");
    std::fs::create_dir(state.join("1.job"))?;
    assert!(jobs.accept(original.clone()).await.is_err());
    assert_eq!(
        jobs.status(Some(1)).await?[0].phase,
        DeployPhase::Interrupted
    );
    assert!(jobs.accept(original).await.is_err());
    assert!(jobs.accept(request("new")).await.is_err());
    jobs.shutdown().await;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_public_permissions_and_symlinked_state() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    std::fs::create_dir(&state)?;
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755))?;
    assert!(open(&state).await.is_err());
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))?;
    let linked = directory.path().join("linked");
    symlink(&state, &linked)?;
    assert!(open(&linked).await.is_err());
    symlink(
        directory.path().join("not-a-ledger"),
        state.join("snapshot.pending"),
    )?;
    assert!(open(&state).await.is_err());
    Ok(())
}

#[tokio::test]
async fn mid_execution_log_failure_cancels_worker_and_blocks_following_job() -> Result<()> {
    use super::tests::GatedExecutor;
    use tokio::sync::{Semaphore, mpsc};
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let jobs = DeploymentJobs::open(
        Arc::new(GatedExecutor {
            started: sender,
            release: release.clone(),
        }),
        state.clone(),
        "fixture".into(),
    )
    .await?;
    let first = jobs.accept(request("first")).await?;
    tokio::time::timeout(std::time::Duration::from_secs(3), receiver.recv())
        .await?
        .unwrap();
    let second = jobs.accept(request("second")).await?;
    std::fs::create_dir(state.join("snapshot.pending"))?;
    release.add_permits(1);
    assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Interrupted);
    assert_eq!(
        terminal(&jobs, second).await.phase,
        DeployPhase::Interrupted
    );
    assert!(receiver.try_recv().is_err());
    assert!(jobs.accept(request("later")).await.is_err());
    jobs.shutdown().await;
    drop(jobs);
    std::fs::remove_dir(state.join("snapshot.pending"))?;
    let restarted = open(&state).await?;
    assert!(
        restarted
            .status(None)
            .await?
            .iter()
            .all(|status| status.phase == DeployPhase::Interrupted)
    );
    restarted.shutdown().await;
    Ok(())
}

// Durable stress fixtures include hundreds of fsyncs; do not inherit the
// volatile executor's three-second scheduling budget. Still fail boundedly.
async fn terminal(jobs: &DeploymentJobs, id: JobId) -> JobStatus {
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let status = jobs.status(Some(id)).await.unwrap().remove(0);
            if status.phase.is_terminal() {
                return status;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("durable fixture should finish within its disk-I/O budget")
}

#[tokio::test]
async fn legacy_control_fragments_load_without_rewriting_history() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let (storage, _) = storage::JobStorage::open(state.clone(), "fixture".into()).await?;
    let original = request("ok");
    let id = original.request_id;
    let mut record = JobRecord::new(original.clone(), 1, CancellationToken::new());
    record.status.phase = DeployPhase::Succeeded;
    record.append(LogSource::Stdout, "placeholder");
    // Valid under the old v1 writer, where ESC/CR were removed bytewise and
    // Unicode direction marks survived. Do not invent or rewrite lost content.
    let legacy = "[32mBuilding[0m[2KDone\u{202e}";
    record.entries[0].text = legacy.into();
    storage.reserve_id(id).await?;
    storage.save(&record).await?;
    drop(storage);
    let path = state.join(format!("{id}.job"));
    let before = std::fs::read(&path)?;
    let recovered = open(&state).await?;
    assert_eq!(recovered.accept(original).await?, id);
    assert_eq!(recovered.logs(id, 0).await?.entries[0].text, legacy);
    assert_eq!(
        crate::utils::plain_text::bounded(legacy, 1024),
        "[32mBuilding[0m[2KDone"
    );
    recovered.shutdown().await;
    drop(recovered);
    assert_eq!(std::fs::read(path)?, before);
    Ok(())
}
