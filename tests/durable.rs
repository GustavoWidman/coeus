//! Abrupt process exit/restart fixture. Only injected fake execution; never nh.
use coeus::{
    common::proto::{DeployPhase, DeployRequest, LogSource},
    jobs::{DeploymentExecutor, DeploymentJobs, JobContext},
};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{process::Command, sync::oneshot, time::timeout};

const REQUEST_ID: u64 = 123456;
fn request() -> DeployRequest {
    DeployRequest {
        request_id: REQUEST_ID,
        rev: "fixture".into(),
        dry_run: false,
        clean_substituters: false,
    }
}
struct NeverExecute;
impl DeploymentExecutor for NeverExecute {
    fn execute(&self, _: DeployRequest, _: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { panic!("recovered job must never execute") })
    }
}
struct CrashFixture(std::sync::Mutex<Option<oneshot::Sender<()>>>);
impl DeploymentExecutor for CrashFixture {
    fn execute(&self, _: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.phase(DeployPhase::Switch).await?;
            context
                .log(
                    LogSource::Stdout,
                    "fake switch started, real nh never executed",
                )
                .await;
            if let Some(sender) = self.0.lock().unwrap().take() {
                let _ = sender.send(());
            }
            std::future::pending().await
        })
    }
}

// Ignored by normal discovery, invoked only as a separate fixture subprocess.
#[tokio::test]
#[ignore]
async fn abrupt_exit_fixture_child() -> Result<()> {
    let Some(directory) = std::env::var_os("COEUS_FIXTURE_STATE") else {
        return Ok(());
    };
    let (sender, receiver) = oneshot::channel();
    let jobs = DeploymentJobs::open(
        Arc::new(CrashFixture(std::sync::Mutex::new(Some(sender)))),
        directory.into(),
        "fixture".into(),
    )
    .await?;
    jobs.accept(request()).await?;
    receiver.await?;
    // No shutdown, drops, or cancellation. Simulate kill after persisted Switch/log.
    std::process::exit(0);
}

#[tokio::test]
async fn abrupt_exit_retains_admission_and_logs_but_never_replays_uncertain_switch() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let state = directory.path().join("state");
    let output = timeout(
        Duration::from_secs(10),
        Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "abrupt_exit_fixture_child",
                "--ignored",
                "--nocapture",
            ])
            .env("COEUS_FIXTURE_STATE", &state)
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let recovered =
        DeploymentJobs::open(Arc::new(NeverExecute), state.clone(), "fixture".into()).await?;
    assert_eq!(recovered.accept(request()).await?, REQUEST_ID);
    assert_eq!(
        recovered.status(Some(REQUEST_ID)).await?[0].phase,
        DeployPhase::Interrupted
    );
    let logs = recovered.logs(REQUEST_ID, 0).await?;
    assert!(logs.finished);
    assert!(
        logs.entries
            .iter()
            .any(|entry| entry.text.contains("real nh never executed"))
    );
    let mut new_request = request();
    new_request.request_id += 1;
    assert!(recovered.accept(new_request).await.is_err());
    recovered.shutdown().await;
    assert!(Path::new(&state).join(format!("{REQUEST_ID}.job")).exists());
    Ok(())
}
