use super::*;
use crate::common::proto::{DeployPhase, DeployRequest, JobId, JobStatus, LogSource};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::timeout;

pub(super) fn request(rev: &str) -> DeployRequest {
    DeployRequest {
        request_id: rand::random(),
        rev: rev.into(),
        dry_run: false,
        clean_substituters: false,
    }
}

pub(super) struct GatedExecutor {
    pub(super) started: mpsc::UnboundedSender<String>,
    pub(super) release: Arc<Semaphore>,
}
impl DeploymentExecutor for GatedExecutor {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.phase(DeployPhase::Build).await?;
            self.started.send(request.rev.clone())?;
            tokio::select! {
                _ = context.cancelled() => context.check_cancelled(),
                permit = self.release.acquire() => {
                    permit?.forget();
                    context.log(LogSource::Stdout, &request.rev).await;
                    Ok(())
                }
            }
        })
    }
}
fn gated() -> (
    DeploymentJobs,
    mpsc::UnboundedReceiver<String>,
    Arc<Semaphore>,
) {
    let (started, receiver) = mpsc::unbounded_channel();
    let release = Arc::new(Semaphore::new(0));
    let jobs = DeploymentJobs::new(Arc::new(GatedExecutor {
        started,
        release: release.clone(),
    }));
    (jobs, receiver, release)
}
pub(super) async fn terminal(jobs: &DeploymentJobs, id: JobId) -> JobStatus {
    timeout(Duration::from_secs(3), async {
        loop {
            let status = jobs.status(Some(id)).await.unwrap().remove(0);
            if status.phase.is_terminal() {
                return status;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("job should become terminal")
}

#[tokio::test]
async fn accepts_while_busy_runs_fifo_and_cancels_queued_work() -> Result<()> {
    timeout(Duration::from_secs(3), async {
        let (jobs, mut started, release) = gated();
        let first = jobs.accept(request("first")).await?;
        assert_eq!(started.recv().await.as_deref(), Some("first"));
        let second = jobs.accept(request("second")).await?;
        let third = jobs.accept(request("third")).await?;
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        jobs.cancel(second).await?;
        release.add_permits(1);
        assert_eq!(started.recv().await.as_deref(), Some("third"));
        assert_eq!(terminal(&jobs, first).await.phase, DeployPhase::Succeeded);
        assert_eq!(terminal(&jobs, second).await.phase, DeployPhase::Cancelled);
        release.add_permits(1);
        assert_eq!(terminal(&jobs, third).await.phase, DeployPhase::Succeeded);
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn queue_full_rejects_without_orphan_ledger_entry() -> Result<()> {
    timeout(Duration::from_secs(3), async {
        let (jobs, mut started, _) = gated();
        let first = jobs.accept(request("running")).await?;
        started.recv().await.unwrap();
        for _ in 0..QUEUE_CAPACITY {
            jobs.accept(request("queued")).await?;
        }
        assert!(jobs.accept(request("overflow")).await.is_err());
        assert_eq!(jobs.status(None).await?.len(), QUEUE_CAPACITY + 1);
        jobs.cancel(first).await?;
        jobs.shutdown().await;
        assert!(
            jobs.status(None)
                .await?
                .iter()
                .all(|job| job.phase == DeployPhase::Cancelled)
        );
        assert!(jobs.accept(request("late")).await.is_err());
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn rejects_bad_input_before_admission_and_reports_unknown_handles() -> Result<()> {
    let (jobs, _, _) = gated();
    for revision in ["".to_owned(), "x".repeat(257), "bad\nrev".into()] {
        assert!(jobs.accept(request(&revision)).await.is_err());
    }
    assert!(jobs.status(None).await?.is_empty());
    assert!(jobs.status(Some(9)).await.is_err());
    assert!(jobs.logs(9, 0).await.is_err());
    assert!(jobs.cancel(9).await.is_err());
    jobs.shutdown().await;
    Ok(())
}

pub(super) struct QuickExecutor;
impl DeploymentExecutor for QuickExecutor {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            match request.rev.as_str() {
                "panic" => panic!("isolated executor panic"),
                "fail" => {
                    eyre::bail!("isolated failure");
                }
                "long-fail" => {
                    eyre::bail!("{}\n\x1b[31m", "á".repeat(2000));
                }
                "logs" => {
                    for _ in 0..300 {
                        context
                            .log(LogSource::Stdout, &format!("\x1b[31m{}", "á".repeat(2000)))
                            .await;
                    }
                    Ok(())
                }
                _ => Ok(()),
            }
        })
    }
}

#[tokio::test]
async fn failure_cause_is_visible_in_bounded_attachment_logs() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(QuickExecutor));
    for rev in ["fail", "long-fail"] {
        let id = jobs.accept(request(rev)).await?;
        let status = terminal(&jobs, id).await;
        let message = status.message.as_ref().unwrap();
        assert!(message.len() <= 1024);
        let logs = jobs.logs(id, 0).await?;
        let entry = logs.entries.last().unwrap();
        assert!(matches!(entry.source, LogSource::Lifecycle));
        assert!(entry.text.starts_with("Failed — "));
        assert!(entry.text.len() <= 1024);
        assert!(!entry.text.chars().any(char::is_control));
        if rev == "fail" {
            assert_eq!(entry.text, "Failed — isolated failure");
        }
    }
    jobs.shutdown().await;
    Ok(())
}

struct MissingCommandExecutor;
impl DeploymentExecutor for MissingCommandExecutor {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            crate::deploy::command::CommandRunner::new(request.rev)
                .envs([("COEUS_FIXTURE_PRIVATE", "not-for-logs")])
                .call_with_output(&context)
                .await
        })
    }
}

#[tokio::test]
async fn failed_spawn_is_inspectable_without_child_output() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let missing = dir.path().join("coeus-fixture-missing-command");
    let jobs = DeploymentJobs::new(Arc::new(MissingCommandExecutor));
    let id = jobs.accept(request(&missing.to_string_lossy())).await?;
    let status = terminal(&jobs, id).await;
    assert_eq!(status.phase, DeployPhase::Failed);
    let message = status.message.as_ref().unwrap();
    assert!(message.contains("starting deployment command"));
    assert!(message.contains("coeus-fixture-missing-command"));
    assert!(message.contains("No such file or directory"));
    assert!(!message.contains("not-for-logs"));
    let logs = jobs.logs(id, 0).await?;
    assert!(
        logs.entries
            .iter()
            .all(|entry| matches!(entry.source, LogSource::Lifecycle))
    );
    assert_eq!(
        logs.entries.last().unwrap().text,
        format!("Failed — {message}")
    );
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn failure_and_panic_do_not_strand_following_deployments() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(QuickExecutor));
    for rev in ["fail", "panic", "ok"] {
        let id = jobs.accept(request(rev)).await?;
        let result = terminal(&jobs, id).await;
        assert_eq!(
            result.phase,
            if rev == "ok" {
                DeployPhase::Succeeded
            } else {
                DeployPhase::Failed
            }
        );
    }
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn bounded_logs_have_cursor_gap_and_terminal_pages_still_drain() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(QuickExecutor));
    let id = jobs.accept(request("logs")).await?;
    terminal(&jobs, id).await;
    let first = jobs.logs(id, 0).await?;
    assert!(first.gap && first.finished);
    assert_eq!(first.entries.len(), ledger::LOG_PAGE);
    assert!(
        first
            .entries
            .iter()
            .all(|entry| entry.text.len() <= ledger::MAX_TEXT && !entry.text.contains('\x1b'))
    );
    let mut after = first.next_cursor;
    let mut count = first.entries.len();
    loop {
        let page = jobs.logs(id, after).await?;
        assert!(!page.gap);
        if page.entries.is_empty() {
            break;
        }
        assert!(page.next_cursor > after);
        count += page.entries.len();
        after = page.next_cursor;
    }
    assert_eq!(count, ledger::MAX_LOGS);
    assert!(jobs.logs(id, after + 1).await.is_err());
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn history_evicts_only_finished_jobs_and_restart_does_not_replay() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(QuickExecutor));
    let first = jobs.accept(request("ok")).await?;
    terminal(&jobs, first).await;
    for _ in 0..ledger::MAX_JOBS {
        let id = jobs.accept(request("ok")).await?;
        terminal(&jobs, id).await;
    }
    assert_eq!(jobs.status(None).await?.len(), ledger::MAX_JOBS);
    assert!(jobs.status(Some(first)).await.is_err());
    jobs.shutdown().await;
    let restarted = DeploymentJobs::new(Arc::new(QuickExecutor));
    assert!(restarted.status(None).await?.is_empty());
    assert!(restarted.status(Some(first)).await.is_err());
    restarted.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn status_pages_are_bounded_ordered_and_single_job_cursor_is_invalid() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(QuickExecutor));
    let mut accepted = Vec::new();
    for _ in 0..70 {
        let id = jobs.accept(request("ok")).await?;
        terminal(&jobs, id).await;
        accepted.push(id);
    }
    let mut after = None;
    let mut listed = Vec::new();
    loop {
        let page = jobs.status_page(None, after).await?;
        assert!(page.jobs.len() <= 32);
        listed.extend(page.jobs.into_iter().map(|job| job.job_id));
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(listed, accepted);
    assert!(
        jobs.status_page(Some(accepted[0]), Some(accepted[1]))
            .await
            .is_err()
    );
    assert!(jobs.status_page(None, Some(0)).await.is_err());
    jobs.shutdown().await;
    Ok(())
}

struct InvalidContextExecutor;
impl DeploymentExecutor for InvalidContextExecutor {
    fn execute(&self, _: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            assert!(context.phase(DeployPhase::Succeeded).await.is_err());
            assert!(
                context
                    .checked_out("unbounded-or-invalid".repeat(100))
                    .await
                    .is_err()
            );
            context.checked_out("a".repeat(40)).await?;
            context.phase(DeployPhase::Build).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn executor_cannot_mark_terminal_or_inflate_commit_payload() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(InvalidContextExecutor));
    let id = jobs.accept(request("ok")).await?;
    assert_eq!(terminal(&jobs, id).await.phase, DeployPhase::Succeeded);
    assert_eq!(
        jobs.status(Some(id)).await?[0].commit.as_deref(),
        Some("a".repeat(40).as_str())
    );
    jobs.shutdown().await;
    Ok(())
}

struct RetainedContextExecutor(mpsc::UnboundedSender<JobContext>);
impl DeploymentExecutor for RetainedContextExecutor {
    fn execute(&self, _: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.0.send(context)?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn finished_log_marker_remains_final_even_if_executor_retains_context() -> Result<()> {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let jobs = DeploymentJobs::new(Arc::new(RetainedContextExecutor(sender)));
    let id = jobs.accept(request("ok")).await?;
    terminal(&jobs, id).await;
    let context = receiver.recv().await.unwrap();
    let before = jobs.logs(id, 0).await?;
    assert!(before.finished);
    context
        .log(LogSource::Stdout, "late output must not appear")
        .await;
    assert!(context.phase(DeployPhase::Build).await.is_err());
    assert!(context.checked_out("b".repeat(40)).await.is_err());
    let after = jobs.logs(id, before.next_cursor).await?;
    assert!(after.finished && after.entries.is_empty());
    jobs.shutdown().await;
    Ok(())
}
