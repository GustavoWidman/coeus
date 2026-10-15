//! Isolated subprocesses: signal delivery must never terminate the test runner.
#![cfg(unix)]
use coeus::{
    common::proto::DeployRequest,
    jobs::{DeploymentExecutor, JobContext},
    server::CoeusServer,
};
use eyre::Result;
use futures_util::future::BoxFuture;
use nix::{
    sys::signal::{Signal, kill},
    unistd::getpid,
};
use std::{sync::Arc, time::Duration};
use tokio::{process::Command, time::timeout};

const CHILD_SIGNAL: &str = "COEUS_SIGNAL_READINESS_TEST_SIGNAL";

struct NoDeploy;
impl DeploymentExecutor for NoDeploy {
    fn execute(&self, _: DeployRequest, _: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async {
            eyre::bail!("signal fixture must not execute deployments");
        })
    }
}

struct SignalAtReadiness;
impl log::Log for SignalAtReadiness {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if record
            .args()
            .to_string()
            .starts_with("coeus server listening on ")
        {
            let signal = match std::env::var(CHILD_SIGNAL).unwrap().as_str() {
                "TERM" => Signal::SIGTERM,
                "INT" => Signal::SIGINT,
                _ => panic!("unknown fixture signal"),
            };
            println!("SIGNAL_AT_READINESS: {signal:?}");
            kill(getpid(), signal).unwrap();
        }
    }
    fn flush(&self) {}
}
static LOGGER: SignalAtReadiness = SignalAtReadiness;

// A current-thread runtime makes the ordering deterministic: a spawned handler
// cannot poll before the synchronous readiness logger sends the signal.
#[tokio::test(flavor = "current_thread")]
async fn server_signal_readiness_child() -> Result<()> {
    if std::env::var_os(CHILD_SIGNAL).is_none() {
        return Ok(());
    }
    log::set_logger(&LOGGER)?;
    log::set_max_level(log::LevelFilter::Info);
    CoeusServer::with_executor("127.0.0.1:0".parse()?, &[0x42; 32], Arc::new(NoDeploy))
        .await?
        .run()
        .await
}

#[tokio::test]
async fn readiness_implies_immediate_sigterm_and_sigint_are_handled() -> Result<()> {
    for signal in ["TERM", "INT"] {
        let output = timeout(
            Duration::from_secs(10),
            Command::new(std::env::current_exe()?)
                .args(["--exact", "server_signal_readiness_child", "--nocapture"])
                .env(CHILD_SIGNAL, signal)
                .kill_on_drop(true)
                .output(),
        )
        .await??;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains("SIGNAL_AT_READINESS:"),
            "child did not reach readiness: {stdout}\n{stderr}"
        );
        assert!(
            output.status.success(),
            "{signal} at readiness exited {}: {stdout}\n{stderr}",
            output.status
        );
    }
    Ok(())
}
