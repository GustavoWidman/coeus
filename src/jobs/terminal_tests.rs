//! Raw retention, volatile eviction and owned PTY cleanup fixtures.
use super::*;
use crate::jobs::{
    DeploymentExecutor, DeploymentJobs, JobContext,
    ledger::MAX_JOBS,
    tests::{GatedExecutor, request, terminal},
};
use futures_util::future::BoxFuture;
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc};

struct RawExecutor;
impl DeploymentExecutor for RawExecutor {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.terminal_bytes(&vec![request.request_id as u8; RETAIN_BYTES])?;
            Ok(())
        })
    }
}

async fn assert_retention(jobs: &DeploymentJobs) {
    // Same lock order as admission/query: ledger -> terminal map -> output.
    let ledger = jobs.ledger.lock().await;
    let terminals = jobs.terminals.lock().await;
    assert_eq!(terminals.len(), ledger.records.len());
    assert!(terminals.len() <= MAX_JOBS);
    let mut bytes = 0;
    for record in &ledger.records {
        bytes += terminals[&record.status.job_id].lock().unwrap().bytes.len();
    }
    assert!(bytes <= MAX_JOBS * RETAIN_BYTES);
}

#[tokio::test]
async fn volatile_eviction_bounds_raw_history_across_completed_jobs() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let jobs = DeploymentJobs::new(Arc::new(RawExecutor));
        let mut first = None;
        for id in 1..=(MAX_JOBS * 2 + 5) as u64 {
            jobs.accept(DeployRequest {
                request_id: id,
                ..request("raw")
            })
            .await?;
            assert_eq!(terminal(&jobs, id).await.phase, DeployPhase::Succeeded);
            if id == 1 {
                first = Some(Arc::downgrade(&jobs.terminals.lock().await[&id]));
            }
            assert_retention(&jobs).await;
            let oldest = id.saturating_sub(MAX_JOBS as u64 - 1).max(1);
            let page = jobs
                .terminal(TerminalRequest {
                    job_id: oldest,
                    after: Some(0),
                    size: Default::default(),
                })
                .await?;
            assert!(page.finished && page.available && !page.gap);
            assert_eq!(page.bytes, vec![oldest as u8; TERMINAL_PAGE_BYTES]);
            if id > MAX_JOBS as u64 {
                let expired = id - MAX_JOBS as u64;
                assert!(jobs.status(Some(expired)).await.is_err());
                assert!(
                    jobs.terminal(TerminalRequest {
                        job_id: expired,
                        after: Some(0),
                        size: Default::default(),
                    })
                    .await
                    .is_err()
                );
                assert!(first.as_ref().unwrap().upgrade().is_none());
            }
        }
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn volatile_eviction_preserves_active_queued_and_retained_terminal_handles() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (started, mut receiver) = mpsc::unbounded_channel();
        let release = Arc::new(Semaphore::new(0));
        let jobs = DeploymentJobs::new(Arc::new(GatedExecutor {
            started,
            release: release.clone(),
        }));
        for id in 1..=MAX_JOBS as u64 {
            jobs.accept(DeployRequest {
                request_id: id,
                ..request("completed")
            })
            .await?;
            receiver.recv().await.unwrap();
            release.add_permits(1);
            terminal(&jobs, id).await;
        }
        let retained = jobs.terminals.lock().await[&(MAX_JOBS as u64)].clone();
        retained.lock().unwrap().push(b"retained")?;
        let running = MAX_JOBS as u64 + 1;
        jobs.accept(DeployRequest {
            request_id: running,
            ..request("running")
        })
        .await?;
        receiver.recv().await.unwrap();
        let active = jobs.terminals.lock().await[&running].clone();
        active.lock().unwrap().push(b"active")?;
        let queued = running + 1;
        jobs.accept(DeployRequest {
            request_id: queued,
            ..request("queued")
        })
        .await?;
        let waiting = jobs.terminals.lock().await[&queued].clone();
        jobs.accept(DeployRequest {
            request_id: queued + 1,
            ..request("extra")
        })
        .await?;
        assert_retention(&jobs).await;
        {
            let map = jobs.terminals.lock().await;
            assert!(Arc::ptr_eq(&retained, &map[&(MAX_JOBS as u64)]));
            assert!(Arc::ptr_eq(&active, &map[&running]));
            assert!(Arc::ptr_eq(&waiting, &map[&queued]));
            assert_eq!(active.lock().unwrap().bytes, b"active");
            assert_eq!(retained.lock().unwrap().bytes, b"retained");
            for id in 1..=3 {
                assert!(!map.contains_key(&id));
            }
        }
        assert_eq!(
            jobs.status(Some(running)).await?[0].phase,
            DeployPhase::Build
        );
        assert_eq!(
            jobs.status(Some(queued)).await?[0].phase,
            DeployPhase::Queued
        );
        jobs.shutdown().await;
        assert_retention(&jobs).await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[test]
fn bounded_raw_history_reports_gaps_and_never_replaces_bytes() -> Result<()> {
    let mut output = TerminalOutput::default();
    let raw: Vec<_> = (0..RETAIN_BYTES + 57).map(|n| n as u8).collect();
    output.push(&raw)?;
    assert_eq!(output.bytes.len(), RETAIN_BYTES);
    let request = |after| TerminalRequest {
        job_id: 4,
        after,
        size: Default::default(),
    };
    let gap = output.page(&request(Some(0)), false, None)?;
    assert!(gap.gap && gap.bytes.is_empty());
    assert_eq!(gap.next_cursor, 57);
    let mut cursor = 57;
    let mut received = vec![];
    loop {
        let page = output.page(&request(Some(cursor)), true, None)?;
        assert!(!page.gap && page.bytes.len() <= TERMINAL_PAGE_BYTES);
        if page.bytes.is_empty() {
            break;
        }
        cursor = page.next_cursor;
        received.extend(page.bytes);
    }
    assert_eq!(received, raw[57..]);
    let live = output.page(&request(None), false, None)?;
    assert_eq!(live.next_cursor, raw.len() as u64);
    assert!(live.bytes.is_empty());
    assert!(
        output
            .page(&request(Some(live.next_cursor + 1)), false, None)
            .is_err()
    );
    assert!(
        output
            .page(
                &TerminalRequest {
                    size: TerminalSize { rows: 0, cols: 80 },
                    ..request(None)
                },
                false,
                None
            )
            .is_err()
    );
    Ok(())
}

#[cfg(all(test, unix))]
mod cleanup_tests {
    use super::*;
    use crate::jobs::{
        DeploymentJobs,
        process_tests::{ShellFixture, finished},
        tests::request,
    };
    use std::time::Duration;
    #[tokio::test]
    async fn cancellation_deadline_and_shutdown_close_master_handles() -> Result<()> {
        tokio::time::timeout(Duration::from_secs(6), async {
            for mode in ["cancel", "deadline", "shutdown"] {
                let jobs = DeploymentJobs::new(Arc::new(ShellFixture {
                    script: "printf 'ready\\n'; sleep 30",
                    deadline: if mode == "deadline" {
                        Duration::from_millis(200)
                    } else {
                        Duration::from_secs(30)
                    },
                }));
                let id = jobs.accept(request(mode)).await?;
                let state = jobs.terminals.lock().await.get(&id).unwrap().clone();
                loop {
                    if !state.lock().unwrap().bytes.is_empty() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                assert!(state.lock().unwrap().master.is_some());
                match mode {
                    "cancel" => {
                        jobs.cancel(id).await?;
                        assert_eq!(finished(&jobs, id).await?, DeployPhase::Cancelled);
                    }
                    "deadline" => {
                        assert_eq!(finished(&jobs, id).await?, DeployPhase::Failed);
                    }
                    _ => {
                        jobs.shutdown().await;
                        assert_eq!(finished(&jobs, id).await?, DeployPhase::Cancelled);
                    }
                }
                assert!(state.lock().unwrap().master.is_none());
                assert!(state.lock().unwrap().group.is_none());
                jobs.shutdown().await;
            }
            Ok::<_, eyre::Report>(())
        })
        .await??;
        Ok(())
    }
}
