//! Exercise the compiled CLI over Noise against an injected executor, never nh.
use coeus::{
    common::proto::{DeployPhase, DeployRequest, LogSource},
    jobs::{DeploymentExecutor, JobContext},
    server::CoeusServer,
};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::{process::Command, time::timeout};
use tokio_util::sync::CancellationToken;
const TEST_PSK: [u8; 32] = [0x42; 32];
struct Fixture;
impl DeploymentExecutor for Fixture {
    fn execute(&self, _: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.phase(DeployPhase::Build).await?;
            context
                .log(LogSource::Stdout, "fixture compiler output")
                .await;
            context.cancelled().await;
            context.check_cancelled()
        })
    }
}
async fn cli(config: &std::path::Path, args: &[&str]) -> Result<std::process::Output> {
    timeout(
        Duration::from_secs(3),
        Command::new(env!("CARGO_BIN_EXE_coeus"))
            .arg("--config")
            .arg(config)
            .args(args)
            .env("NO_COLOR", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await?
    .map_err(Into::into)
}

#[tokio::test]
async fn compiled_cli_accepts_inspects_attaches_cancels_and_reports_rejection() -> Result<()> {
    timeout(Duration::from_secs(10), async {
        let server =
            CoeusServer::with_executor("127.0.0.1:0".parse()?, &TEST_PSK, Arc::new(Fixture))
                .await?;
        let address = server.local_addr()?;
        let dir = tempfile::tempdir()?;
        let config = dir.path().join("client.toml");
        std::fs::write(
            &config,
            format!(
                "[settings]\nkey = \"{}\"\nhost = \"127.0.0.1\"\nport = {}\n",
                "42".repeat(32),
                address.port()
            ),
        )?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(server.run_until_cancelled(shutdown.clone()));
        let accepted = cli(&config, &["deploy", "--rev", "fixture"]).await?;
        assert!(
            accepted.status.success(),
            "{}",
            String::from_utf8_lossy(&accepted.stderr)
        );
        let stdout = String::from_utf8_lossy(&accepted.stdout);
        let line = stdout
            .lines()
            .find(|line| line.starts_with("accepted "))
            .expect("ack must show job handle");
        let id = line.split_whitespace().nth(1).unwrap().to_owned();
        let status = cli(&config, &["status", &id]).await?;
        assert!(status.status.success());
        assert!(String::from_utf8_lossy(&status.stdout).contains("Build"));
        let logs = cli(&config, &["logs", &id]).await?;
        assert!(logs.status.success());
        assert!(String::from_utf8_lossy(&logs.stdout).contains("fixture compiler output"));
        let cancel = cli(&config, &["cancel", &id]).await?;
        assert!(cancel.status.success());
        let terminal = cli(&config, &["status", &id, "--watch"]).await?;
        assert!(terminal.status.success());
        assert!(String::from_utf8_lossy(&terminal.stdout).contains("Cancelled"));
        let invalid = cli(&config, &["status", "0"]).await?;
        assert!(!invalid.status.success());
        assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown or expired"));
        shutdown.cancel();
        task.await??;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
