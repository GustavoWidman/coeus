//! Only harmless shell fixtures. These tests never construct Deployer or execute nh.
use super::{DeploymentExecutor, DeploymentJobs, JobContext};
use crate::{
    common::proto::{DeployPhase, DeployRequest, JobId, LogSource},
    deploy::command::CommandRunner,
};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

pub(super) struct ShellFixture {
    pub(super) script: &'static str,
    pub(super) deadline: Duration,
}
impl DeploymentExecutor for ShellFixture {
    fn execute(&self, _: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.phase(DeployPhase::Build).await?;
            CommandRunner::new("sh")
                .args(["-c", self.script, "sh"])
                .call_with_deadline(&context, self.deadline)
                .await
        })
    }
}
fn request() -> DeployRequest {
    DeployRequest {
        request_id: rand::random(),
        rev: "fake-shell".into(),
        dry_run: false,
        clean_substituters: false,
    }
}
pub(super) async fn finished(jobs: &DeploymentJobs, id: JobId) -> Result<DeployPhase> {
    timeout(Duration::from_secs(3), async {
        loop {
            let status = jobs.status(Some(id)).await?.remove(0);
            if status.phase.is_terminal() {
                return Ok::<_, eyre::Report>(status.phase);
            }
            tokio::task::yield_now().await;
        }
    })
    .await?
}
pub(super) async fn output(jobs: &DeploymentJobs, id: JobId) -> Result<Vec<(LogSource, String)>> {
    let mut cursor = 0;
    let mut all = Vec::new();
    loop {
        let page = jobs.logs(id, cursor).await?;
        if page.entries.is_empty() {
            break;
        }
        cursor = page.next_cursor;
        all.extend(
            page.entries
                .into_iter()
                .map(|entry| (entry.source, entry.text)),
        );
    }
    Ok(all)
}

#[tokio::test]
async fn drains_both_streams_with_bounded_chunks_even_without_newlines() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(ShellFixture {
        script: "printf 'stdout-mark\n'; printf 'stderr-mark\n' >&2; i=0; while [ $i -lt 12000 ]; do printf x; i=$((i+1)); done",
        deadline: Duration::from_secs(2),
    }));
    let id = jobs.accept(request()).await?;
    assert_eq!(finished(&jobs, id).await?, DeployPhase::Succeeded);
    let lines = output(&jobs, id).await?;
    assert!(
        lines
            .iter()
            .any(|(source, text)| matches!(source, LogSource::Terminal) && text == "stdout-mark")
    );
    assert!(
        lines
            .iter()
            .any(|(source, text)| matches!(source, LogSource::Terminal) && text == "stderr-mark")
    );
    assert!(lines.iter().all(|(_, text)| text.len() <= 1024));
    assert_eq!(
        lines
            .iter()
            .filter(|(_, text)| text.starts_with('x'))
            .map(|(_, text)| text.len())
            .sum::<usize>(),
        12000
    );
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn nonzero_exit_keeps_tail_logs_and_failed_status() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(ShellFixture {
        script: "printf 'failure tail\n' >&2; exit 7",
        deadline: Duration::from_secs(2),
    }));
    let id = jobs.accept(request()).await?;
    assert_eq!(finished(&jobs, id).await?, DeployPhase::Failed);
    assert!(
        output(&jobs, id)
            .await?
            .iter()
            .any(|(_, text)| text == "failure tail")
    );
    assert!(
        jobs.status(Some(id)).await?[0]
            .message
            .as_ref()
            .unwrap()
            .contains('7')
    );
    jobs.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn command_deadline_is_failed_not_successful() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(ShellFixture {
        script: "sleep 30",
        deadline: Duration::from_millis(100),
    }));
    let id = jobs.accept(request()).await?;
    assert_eq!(finished(&jobs, id).await?, DeployPhase::Failed);
    assert!(
        jobs.status(Some(id)).await?[0]
            .message
            .as_ref()
            .unwrap()
            .contains("deadline")
    );
    jobs.shutdown().await;
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn running_cancel_kills_descendant_not_just_shell_parent() -> Result<()> {
    timeout(Duration::from_secs(3), async {
        let jobs = DeploymentJobs::new(Arc::new(ShellFixture {
            script: "sleep 30 & printf '%s\n' \"$!\"; wait",
            deadline: Duration::from_secs(2),
        }));
        let id = jobs.accept(request()).await?;
        let pid = loop {
            if let Some((_, text)) = output(&jobs, id)
                .await?
                .iter()
                .find(|(source, _)| matches!(source, LogSource::Terminal))
            {
                break text.parse::<u32>()?;
            }
            tokio::task::yield_now().await;
        };
        jobs.cancel(id).await?;
        assert_eq!(finished(&jobs, id).await?, DeployPhase::Cancelled);
        // An adopted zombie may briefly remain; it must not be a live descendant.
        loop {
            match tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Ok(stat)
                    if stat
                        .split_once(") ")
                        .is_some_and(|(_, rest)| rest.starts_with('Z')) =>
                {
                    break;
                }
                _ => tokio::task::yield_now().await,
            }
        }
        jobs.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn exec_receives_only_slave_stdio_not_inherited_pty_masters() -> Result<()> {
    let jobs = DeploymentJobs::new(Arc::new(ShellFixture {
        script: r#"for fd in /proc/self/fd/*; do target=$(readlink "$fd" || true); case "$target" in /dev/pts/ptmx|/dev/ptmx) exit 29;; esac; done; printf 'no inherited masters\n'"#,
        deadline: Duration::from_secs(2),
    }));
    let id = jobs.accept(request()).await?;
    assert_eq!(finished(&jobs, id).await?, DeployPhase::Succeeded);
    assert!(
        output(&jobs, id)
            .await?
            .iter()
            .any(|(_, text)| text == "no inherited masters")
    );
    jobs.shutdown().await;
    Ok(())
}
