//! Fake backend regressions for fleet protocol, persistent IDs and cross-job feed.
use super::tests::{QuickExecutor, request, terminal};
use super::*;
use crate::common::proto::*;
use eyre::Result;
use std::sync::Arc;

#[tokio::test]
async fn server_assigns_sequential_ids_separate_from_nonce_and_concurrent_retry() -> Result<()> {
    let jobs = Arc::new(DeploymentJobs::new(Arc::new(QuickExecutor)));
    let original = request("ok");
    let (one, retry) = tokio::join!(jobs.accept(original.clone()), jobs.accept(original));
    assert_eq!(one?, 1);
    assert_eq!(retry?, 1);
    terminal(&jobs, 1).await;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..10 {
        let jobs = jobs.clone();
        tasks.spawn(async move { jobs.accept(request("ok")).await });
    }
    let mut ids = vec![];
    while let Some(result) = tasks.join_next().await {
        ids.push(result??);
    }
    ids.sort();
    assert_eq!(ids, (2..=11).collect::<Vec<_>>());
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn high_water_survives_restart_and_missing_archived_records_without_reuse() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state");
    let open = || DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into());
    let jobs = open().await?;
    let original = request("ok");
    assert_eq!(jobs.accept(original.clone()).await?, 1);
    terminal(&jobs, 1).await;
    jobs.shutdown().await;
    drop(jobs);
    let jobs = open().await?;
    assert_eq!(jobs.accept(original).await?, 1);
    assert_eq!(jobs.accept(request("ok")).await?, 2);
    terminal(&jobs, 2).await;
    jobs.shutdown().await;
    drop(jobs);
    // Explicit offline archival fixture: the independent counter must not reset.
    std::fs::remove_file(path.join("2.job"))?;
    let jobs = open().await?;
    assert_eq!(jobs.accept(request("ok")).await?, 3);
    terminal(&jobs, 3).await;
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn burned_id_and_counter_overflow_fail_safely() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state");
    let (storage, _) = storage::JobStorage::open(path.clone(), "fixture".into()).await?;
    storage.reserve_id(10).await?; // crash before writing a job snapshot / ACK
    drop(storage);
    let jobs =
        DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into()).await?;
    assert_eq!(jobs.accept(request("ok")).await?, 11);
    terminal(&jobs, 11).await;
    jobs.shutdown().await;
    drop(jobs);
    std::fs::write(path.join("id-counter"), u64::MAX.to_be_bytes())?;
    let jobs = DeploymentJobs::open(Arc::new(QuickExecutor), path, "fixture".into()).await?;
    assert!(
        jobs.accept(request("ok"))
            .await
            .unwrap_err()
            .to_string()
            .contains("exhausted")
    );
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn legacy_v1_random_handles_and_nonces_migrate_without_rewriting_terminal_history()
-> Result<()> {
    use super::snapshot::LegacySnapshot;
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state");
    // Construct an actual v1-only ledger, without v2 initialization metadata.
    std::fs::create_dir(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut original = request("old");
    original.request_id = 900;
    let old = LegacySnapshot {
        version: 1,
        scope: "fixture".into(),
        sequence: 1,
        request: original.clone(),
        status: JobStatus {
            job_id: 900,
            revision: "old".into(),
            commit: None,
            phase: DeployPhase::Succeeded,
            cancel_requested: false,
            message: None,
        },
        entries: Default::default(),
        cursor: 0,
    };
    let bytes = postcard::to_stdvec(&old)?;
    let file = path.join("900.job");
    std::fs::write(&file, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))?;
    }
    let jobs =
        DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into()).await?;
    assert_eq!(jobs.accept(original).await?, 900);
    assert_eq!(std::fs::read(file)?, bytes);
    assert_eq!(
        std::fs::read(path.join("id-counter"))?,
        900u64.to_be_bytes()
    );
    assert_eq!(
        std::fs::read(path.join("ledger-format"))?,
        b"coeus-ledger-v2\n"
    );
    assert_eq!(jobs.accept(request("ok")).await?, 901);
    terminal(&jobs, 901).await;
    jobs.shutdown().await;
    Ok(())
}

struct Raw;
impl DeploymentExecutor for Raw {
    fn execute(
        &self,
        request: DeployRequest,
        context: JobContext,
    ) -> futures_util::future::BoxFuture<'_, Result<()>> {
        Box::pin(async move { context.terminal_bytes(request.rev.as_bytes()) })
    }
}
#[tokio::test]
async fn daemon_feed_crosses_jobs_without_readers_consuming_each_other() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(Raw));
    let page = |after| SharedTerminalRequest {
        after,
        size: Default::default(),
    };
    let live = jobs.shared_terminal(page(None)).await?;
    assert_eq!(live.next_cursor, 0);
    let first = jobs.accept(request("\x1b[31m")).await;
    assert!(first.is_err()); // deploy revisions cannot carry terminal controls
    let first = jobs.accept(request("one")).await?;
    terminal(&jobs, first).await;
    let a = jobs.shared_terminal(page(Some(live.next_cursor))).await?;
    assert_eq!(a.chunks[0].job_id, first);
    let second = jobs.accept(request("two")).await?;
    terminal(&jobs, second).await;
    let b = jobs.shared_terminal(page(Some(a.next_cursor))).await?;
    assert_eq!(b.chunks[0].job_id, second);
    assert_eq!(b.chunks[0].bytes, b"two");
    let independent = jobs.shared_terminal(page(Some(0))).await?;
    assert_eq!(
        independent
            .chunks
            .iter()
            .map(|c| c.bytes.len())
            .sum::<usize>(),
        6
    );
    let status = jobs.machine_status().await?;
    assert!(status.active.is_none());
    assert_eq!(status.latest.unwrap().job_id, second);
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn corrupted_or_regressed_counter_is_not_silently_repaired() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state");
    let jobs =
        DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into()).await?;
    jobs.accept(request("ok")).await?;
    terminal(&jobs, 1).await;
    jobs.shutdown().await;
    drop(jobs);
    for bytes in [0u64.to_be_bytes().to_vec(), vec![1, 2, 3], vec![0; 4096]] {
        std::fs::write(path.join("id-counter"), bytes)?;
        assert!(
            DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into())
                .await
                .is_err()
        );
    }
    std::fs::remove_file(path.join("1.job"))?;
    for bytes in [vec![1, 2, 3], vec![0; 4096]] {
        std::fs::write(path.join("id-counter"), bytes)?;
        assert!(
            DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into())
                .await
                .is_err()
        );
    }
    Ok(())
}

/// No max(retained) reconstruction: the largest ID may already be archived or
/// reserved without ever writing a snapshot. A missing counter is not migration.
#[tokio::test]
async fn missing_v2_counter_fails_closed_with_retained_or_pruned_history() -> Result<()> {
    for prune in [false, true] {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("state");
        let open = || DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into());
        let jobs = open().await?;
        for id in 1..=2 {
            assert_eq!(jobs.accept(request("ok")).await?, id);
            terminal(&jobs, id).await;
        }
        jobs.shutdown().await;
        drop(jobs);
        std::fs::remove_file(path.join("2.job"))?;
        if prune {
            std::fs::remove_file(path.join("1.job"))?;
        }
        std::fs::remove_file(path.join("id-counter"))?;
        for _ in 0..2 {
            let error = match open().await {
                Ok(jobs) => {
                    jobs.shutdown().await;
                    panic!("missing counter accepted");
                }
                Err(error) => error,
            };
            assert!(error.to_string().contains("missing its ID counter"));
            assert!(!path.join("id-counter").exists());
        }
    }
    Ok(())
}

#[tokio::test]
async fn empty_initialized_and_burned_only_ledgers_reject_lost_counter() -> Result<()> {
    for burned in [None, Some(42)] {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("state");
        let (storage, _) = storage::JobStorage::open(path.clone(), "fixture".into()).await?;
        if let Some(id) = burned {
            storage.reserve_id(id).await?;
        }
        drop(storage);
        // A legitimate fresh empty ledger can restart, even before its first job.
        let (storage, _) = storage::JobStorage::open(path.clone(), "fixture".into()).await?;
        assert_eq!(storage.high_water(), burned.unwrap_or(0));
        drop(storage);
        std::fs::remove_file(path.join("id-counter"))?;
        assert!(
            storage::JobStorage::open(path.clone(), "fixture".into())
                .await
                .is_err()
        );
        assert!(!path.join("id-counter").exists());
    }
    Ok(())
}

#[tokio::test]
async fn v2_history_without_initialization_marker_does_not_become_v1_migration() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state");
    let jobs =
        DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into()).await?;
    assert_eq!(jobs.accept(request("ok")).await?, 1);
    terminal(&jobs, 1).await;
    jobs.shutdown().await;
    drop(jobs);
    std::fs::remove_file(path.join("ledger-format"))?;
    std::fs::remove_file(path.join("id-counter"))?;
    assert!(
        DeploymentJobs::open(Arc::new(QuickExecutor), path.clone(), "fixture".into())
            .await
            .is_err()
    );
    assert!(!path.join("id-counter").exists());
    Ok(())
}

#[tokio::test]
async fn initialization_crash_boundaries_preserve_counter_or_fail_closed() -> Result<()> {
    for pending in ["ledger-format.pending", "id-counter.pending"] {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("state");
        let (storage, _) = storage::JobStorage::open(path.clone(), "fixture".into()).await?;
        storage.reserve_id(42).await?;
        drop(storage);
        std::fs::remove_file(path.join("ledger-format"))?;
        // Counter persisted before marker publication: safe to finish initializing.
        let (storage, _) = storage::JobStorage::open(path.clone(), "fixture".into()).await?;
        assert_eq!(storage.high_water(), 42);
        drop(storage);
        std::fs::rename(path.join("ledger-format"), path.join(pending))?;
        std::fs::remove_file(path.join("id-counter"))?;
        assert!(
            storage::JobStorage::open(path.clone(), "fixture".into())
                .await
                .is_err()
        );
        assert!(!path.join("id-counter").exists());
    }
    Ok(())
}

#[tokio::test]
async fn malformed_format_marker_is_not_silently_reinitialized() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("state");
    let (storage, _) = storage::JobStorage::open(path.clone(), "fixture".into()).await?;
    drop(storage);
    for bytes in [b"coeus-ledger-v1\n".to_vec(), vec![], vec![0; 4096]] {
        std::fs::write(path.join("ledger-format"), bytes)?;
        assert!(
            storage::JobStorage::open(path.clone(), "fixture".into())
                .await
                .is_err()
        );
    }
    Ok(())
}
