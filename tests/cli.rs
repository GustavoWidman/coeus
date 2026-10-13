//! Exercise the compiled CLI over Noise against an injected executor, never nh.
use coeus::{
    common::{
        proto::{DeployPhase, DeployRequest, LogSource, Packet},
        stream::EncryptedListener,
    },
    jobs::{DeploymentExecutor, JobContext},
    server::CoeusServer,
};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::{process::Command, time::timeout};
use tokio_util::sync::CancellationToken;
const TEST_PSK: [u8; 32] = [0x42; 32];
struct Fixture;
impl DeploymentExecutor for Fixture {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if request.rev == "gap-fail" {
                for cursor in 0..300 {
                    context
                        .log(LogSource::Stdout, &format!("fixture output {cursor}"))
                        .await;
                }
                eyre::bail!("fixture checkout failed before child spawn");
            }
            context.phase(DeployPhase::Build).await?;
            context
                .log(LogSource::Stdout, "fixture compiler output")
                .await;
            context
                .log(LogSource::Stderr, "fixture compiler warning")
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
        let accepted = cli(
            &config,
            &["deploy", "--rev", "fixture", "--request-id", "701"],
        )
        .await?;
        assert!(
            accepted.status.success(),
            "{}",
            String::from_utf8_lossy(&accepted.stderr)
        );
        let stdout = String::from_utf8_lossy(&accepted.stdout);
        assert!(stdout.contains("INFO  request-id: 701 (retry with identical options)"));
        assert!(stdout.contains("INFO  accepted 701"));
        assert!(stdout.find("request-id: 701").unwrap() < stdout.find("accepted 701").unwrap());
        let id = "701";
        let status = cli(&config, &["status", id]).await?;
        assert!(status.status.success());
        assert!(String::from_utf8_lossy(&status.stdout).contains("INFO  701 fixture Build"));
        let logs = cli(&config, &["logs", id]).await?;
        assert!(logs.status.success());
        assert!(String::from_utf8_lossy(&logs.stdout).contains("fixture compiler output"));
        let quiet_status = cli(&config, &["--verbosity", "off", "status", id]).await?;
        assert!(quiet_status.status.success());
        assert!(quiet_status.stdout.is_empty());
        let quiet_logs = cli(&config, &["--verbosity", "off", "logs", id]).await?;
        assert!(quiet_logs.status.success());
        let data = String::from_utf8_lossy(&quiet_logs.stdout);
        assert!(data.contains("[stdout] fixture compiler output"));
        assert!(data.contains("[stderr] fixture compiler warning"));
        assert!(!data.contains("[phase]"));
        assert!(!data.contains("INFO"));
        let cancel = cli(&config, &["cancel", id]).await?;
        assert!(cancel.status.success());
        let terminal = cli(&config, &["status", id, "--watch"]).await?;
        assert!(terminal.status.success());
        assert!(String::from_utf8_lossy(&terminal.stdout).contains("Cancelled"));
        let invalid = cli(&config, &["status", "0"]).await?;
        assert!(!invalid.status.success());
        assert!(String::from_utf8_lossy(&invalid.stderr).contains("unknown or expired"));
        let failure = cli(
            &config,
            &["deploy", "--rev", "gap-fail", "--request-id", "702"],
        )
        .await?;
        assert!(failure.status.success());
        let failed = cli(&config, &["status", "702", "--watch"]).await?;
        assert!(failed.status.success());
        assert!(
            String::from_utf8_lossy(&failed.stdout)
                .contains("Failed — fixture checkout failed before child spawn")
        );
        let failure_logs = cli(&config, &["logs", "702", "--follow"]).await?;
        assert!(failure_logs.status.success());
        let output = String::from_utf8_lossy(&failure_logs.stdout);
        assert!(output.contains("WARN  older output was evicted"));
        assert!(output.contains("INFO "));
        assert!(output.contains("[phase] Failed — fixture checkout failed before child spawn"));
        let quiet_logs = cli(&config, &["--verbosity", "off", "logs", "702"]).await?;
        assert!(quiet_logs.status.success());
        let output = String::from_utf8_lossy(&quiet_logs.stdout);
        assert!(output.contains("[stdout] fixture output"));
        assert!(!output.contains("WARN"));
        assert!(!output.contains("[phase]"));
        shutdown.cancel();
        task.await??;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

/// Wait for the flushed retry key before allowing even a fake server to receive
/// the request. Then lose the ACK: quiet verbosity must not hide the handle.
#[tokio::test]
async fn quiet_retry_id_is_visible_and_flushed_before_acknowledgement() -> Result<()> {
    timeout(Duration::from_secs(10), async {
        for verbosity in ["off", "error", "warn"] {
            let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &TEST_PSK).await?;
            let dir = tempfile::tempdir()?;
            let config = dir.path().join("client.toml");
            std::fs::write(
                &config,
                format!(
                    "[settings]\nkey = \"{}\"\nhost = \"127.0.0.1\"\nport = {}\n",
                    "42".repeat(32),
                    listener.local_addr()?.port(),
                ),
            )?;
            let mut child = Command::new(env!("CARGO_BIN_EXE_coeus"))
                .arg("--config")
                .arg(&config)
                .args(["--verbosity", verbosity, "deploy", "--rev", "fixture"])
                .env("NO_COLOR", "1")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;
            let (mut peer, _) = listener.accept().await?;
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            timeout(Duration::from_secs(2), stdout.read_line(&mut line)).await??;
            assert!(line.contains("INFO  request-id: "), "{line:?}");
            let id = line
                .split("request-id: ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse::<u64>()?;
            assert_ne!(id, 0);
            let Some(Packet::DeployRequest(request)) = peer.recv().await? else {
                panic!("expected deployment request");
            };
            assert_eq!(request.request_id, id);
            // No executor, no acceptance and no side effects: just a lost ACK.
            drop(peer);
            let output = child.wait_with_output().await?;
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("server closed"));
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn failed_retry_id_write_prevents_request_submission() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &TEST_PSK).await?;
        let dir = tempfile::tempdir()?;
        let config = dir.path().join("client.toml");
        std::fs::write(
            &config,
            format!(
                "[settings]\nkey = \"{}\"\nhost = \"127.0.0.1\"\nport = {}\n",
                "42".repeat(32),
                listener.local_addr()?.port(),
            ),
        )?;
        let mut child = Command::new(env!("CARGO_BIN_EXE_coeus"))
            .arg("--config")
            .arg(config)
            .args([
                "--verbosity",
                "off",
                "deploy",
                "--rev",
                "fixture",
                "--request-id",
                "703",
            ])
            .env("NO_COLOR", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        // A closed pipe makes the checked retry-key write fail. No deployment
        // envelope may be sent when the handle cannot be presented.
        drop(child.stdout.take());
        let (mut peer, _) = listener.accept().await?;
        assert!(peer.recv().await?.is_none());
        let output = child.wait_with_output().await?;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Broken pipe"));
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}
