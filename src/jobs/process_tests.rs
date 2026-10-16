//! Harmless shell fixtures; activation cases cross the real Deployer orchestration
//! boundary with an injected executor, never checkout/build/activate the host.
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

/// Same ownership contract as the root helper, using only unprivileged harmless
/// processes. The activation child has its own session: killing the outer PGID
/// cannot clean it. stdin EOF must reach the helper and reap its owned group.
struct SupervisedActivationFixture {
    marker: std::path::PathBuf,
    deadline: Duration,
    cooperative: bool,
}
struct FixtureActivation<'a> {
    marker: &'a std::path::Path,
    context: &'a JobContext,
    cooperative: bool,
}
impl crate::activation::ActivationExecutor for FixtureActivation<'_> {
    fn current(&self) -> Result<std::path::PathBuf> {
        Ok("/fixture".into())
    }
    fn validate<'a>(&'a self, _: &'a crate::activation::Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn verify<'a>(&'a self, _: &'a crate::activation::Receipt) -> BoxFuture<'a, Result<()>> {
        Box::pin(async {
            eyre::bail!("fixture must not verify an interrupted activation");
        })
    }
    fn switch<'a>(
        &'a self,
        _: &'a crate::activation::Receipt,
        deadline: Duration,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let program = r#"import importlib.util,os,sys,time
s=importlib.util.spec_from_file_location('p',sys.argv[1]); p=importlib.util.module_from_spec(s); s.loader.exec_module(p)
script='sleep 30 & echo $! > "$1"; wait'
code=p.supervised(['/bin/sh','-c',script,'sh',sys.argv[2]],dict(os.environ),deadline=10)
# Artificial cleanup lag proves production orchestration joins the supervisor.
time.sleep(0.15)
open(sys.argv[2]+'.settled','w').write(str(code))
sys.exit(code)"#;
            let nonack = r#"import os,subprocess,sys,time
child=subprocess.Popen(['/bin/sh','-c','sleep 15'],start_new_session=True)
open(sys.argv[2],'w').write(str(child.pid))
time.sleep(15)"#;
            CommandRunner::new("python3")
                .arg("-c")
                .arg(if self.cooperative { program } else { nonack })
                .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/nix/elevation.py"))
                .arg(self.marker)
                .supervised_activation()
                .call_with_deadline(self.context, deadline)
                .await
        })
    }
}
impl DeploymentExecutor for SupervisedActivationFixture {
    fn execute(&self, _: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            use crate::activation::{ActivationState, ReceiptStore};
            context.phase(DeployPhase::Switch).await?;
            let store = ReceiptStore::open(&context.activation_directory().await?)?;
            let mut receipt = super::handoff_tests::receipt(&store, context.job_id());
            receipt.manual = false;
            receipt.state = ActivationState::Ready;
            store.save(&receipt)?;
            crate::deploy::Deployer::activate_built(
                &store,
                context.job_id(),
                &FixtureActivation {
                    marker: &self.marker,
                    context: &context,
                    cooperative: self.cooperative,
                },
                self.deadline,
            )
            .await
        })
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn supervised_activation_cancel_kills_separate_group_and_preserves_uncertainty() -> Result<()>
{
    timeout(Duration::from_secs(8), async {
        let temp = tempfile::tempdir()?;
        let marker = temp.path().join("activation-child.pid");
        let path = temp.path().join("state");
        let jobs = DeploymentJobs::open(
            Arc::new(SupervisedActivationFixture {
                marker: marker.clone(),
                deadline: Duration::from_secs(12),
                cooperative: true,
            }),
            path.clone(),
            "fixture".into(),
        )
        .await?;
        let first = jobs.accept(request()).await?;
        let pid = loop {
            if let Ok(text) = tokio::fs::read_to_string(&marker).await
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            tokio::task::yield_now().await;
        };
        let second = jobs.accept(request()).await?;
        jobs.cancel(first).await?;
        assert_eq!(finished(&jobs, first).await?, DeployPhase::Interrupted);
        let status = jobs.status(Some(first)).await?.remove(0);
        assert!(status.cancel_requested);
        assert!(status.message.unwrap().contains("outcome uncertain"));
        // The supervised cancellation exits before terminal status is recorded.
        assert!(marker.with_extension("pid.settled").is_file());
        match tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(stat)
                if stat
                    .split_once(") ")
                    .is_some_and(|(_, rest)| rest.starts_with('Z')) => {}
            other => panic!("activation descendant survived acknowledged cancellation: {other:?}"),
        }
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        assert!(jobs.accept(request()).await.is_err());
        let activation = crate::activation::directory_for_ledger(&path)?;
        assert_eq!(
            crate::activation::read_receipt(&activation, first)?.state,
            crate::activation::ActivationState::Uncertain
        );
        jobs.shutdown().await;
        drop(jobs);
        let restarted = DeploymentJobs::open(
            Arc::new(super::tests::QuickExecutor),
            path,
            "fixture".into(),
        )
        .await?;
        assert!(
            restarted
                .status(None)
                .await?
                .iter()
                .all(|s| s.phase == DeployPhase::Interrupted || s.phase == DeployPhase::Queued)
        );
        assert!(restarted.accept(request()).await.is_err());
        restarted.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn production_activation_deadline_joins_supervisor_before_uncertain_barrier() -> Result<()> {
    timeout(Duration::from_secs(8), async {
        let temp = tempfile::tempdir()?;
        let marker = temp.path().join("activation-child.pid");
        let path = temp.path().join("state");
        let jobs = DeploymentJobs::open(
            Arc::new(SupervisedActivationFixture {
                marker: marker.clone(),
                deadline: Duration::from_millis(700),
                cooperative: true,
            }),
            path.clone(),
            "fixture".into(),
        )
        .await?;
        let first = jobs.accept(request()).await?;
        let pid = loop {
            if let Ok(text) = tokio::fs::read_to_string(&marker).await
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            tokio::task::yield_now().await;
        };
        let second = jobs.accept(request()).await?;
        loop {
            if jobs.status(Some(first)).await?[0].phase == DeployPhase::RequiresManualSwitch {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(marker.with_extension("pid.settled").is_file());
        match tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(stat)
                if stat
                    .split_once(") ")
                    .is_some_and(|(_, rest)| rest.starts_with('Z')) => {}
            other => panic!("activation descendant survived deadline settlement: {other:?}"),
        }
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        let activation = crate::activation::directory_for_ledger(&path)?;
        assert_eq!(
            crate::activation::read_receipt(&activation, first)?.state,
            crate::activation::ActivationState::Uncertain
        );
        let third = jobs.accept(request()).await?;
        assert_eq!(
            jobs.status(Some(third)).await?[0].phase,
            DeployPhase::Queued
        );
        jobs.cancel(first).await?;
        assert_eq!(finished(&jobs, first).await?, DeployPhase::Interrupted);
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        assert!(jobs.accept(request()).await.is_err());
        assert_eq!(
            crate::activation::read_receipt(&activation, first)?.state,
            crate::activation::ActivationState::Uncertain
        );
        jobs.shutdown().await;
        drop(jobs);
        let restarted = DeploymentJobs::open(
            Arc::new(super::tests::QuickExecutor),
            path,
            "fixture".into(),
        )
        .await?;
        assert!(
            restarted
                .status(None)
                .await?
                .iter()
                .all(|status| status.phase == DeployPhase::Interrupted
                    || status.phase == DeployPhase::Queued)
        );
        assert!(restarted.accept(request()).await.is_err());
        assert_eq!(
            crate::activation::read_receipt(&activation, first)?.state,
            crate::activation::ActivationState::Uncertain
        );
        restarted.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

/// Cleanup failure is not proof of termination. This harmless detached sleep
/// deliberately models a privileged survivor the outer PGID cannot clean.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn nonacknowledging_supervisor_blocks_fifo_admission_and_restart_replay() -> Result<()> {
    struct FixtureGroup(nix::unistd::Pid);
    impl Drop for FixtureGroup {
        fn drop(&mut self) {
            let _ = nix::sys::signal::killpg(self.0, nix::sys::signal::Signal::SIGKILL);
        }
    }
    timeout(Duration::from_secs(12), async {
        let temp = tempfile::tempdir()?;
        let marker = temp.path().join("nonack-child.pid");
        let path = temp.path().join("state");
        let jobs = DeploymentJobs::open(
            Arc::new(SupervisedActivationFixture {
                marker: marker.clone(),
                deadline: Duration::from_secs(12),
                cooperative: false,
            }),
            path.clone(),
            "fixture".into(),
        )
        .await?;
        let first = jobs.accept(request()).await?;
        let pid = loop {
            if let Ok(text) = tokio::fs::read_to_string(&marker).await
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                break pid;
            }
            tokio::task::yield_now().await;
        };
        let _cleanup = FixtureGroup(nix::unistd::Pid::from_raw(pid));
        let second = jobs.accept(request()).await?;
        jobs.cancel(first).await?;
        timeout(Duration::from_secs(8), async {
            loop {
                if jobs.status(Some(first)).await?[0].phase == DeployPhase::Interrupted {
                    break;
                }
                tokio::task::yield_now().await;
            }
            Ok::<_, eyre::Report>(())
        })
        .await??;
        assert!(
            jobs.status(Some(first)).await?[0]
                .message
                .as_ref()
                .unwrap()
                .contains("supervisor did not settle")
        );
        assert_eq!(
            jobs.status(Some(second)).await?[0].phase,
            DeployPhase::Queued
        );
        assert!(jobs.accept(request()).await.is_err());
        // The modeled survivor really can outlive outer cleanup, without allowing
        // any next deployment past the durable uncertainty barrier.
        let stat = tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await?;
        assert!(!stat.split_once(") ").unwrap().1.starts_with('Z'));
        jobs.shutdown().await;
        drop(jobs);
        let restarted = DeploymentJobs::open(
            Arc::new(super::tests::QuickExecutor),
            path,
            "fixture".into(),
        )
        .await?;
        assert!(
            restarted
                .status(None)
                .await?
                .iter()
                .all(|status| status.phase == DeployPhase::Interrupted
                    || status.phase == DeployPhase::Queued)
        );
        assert!(restarted.accept(request()).await.is_err());
        restarted.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
