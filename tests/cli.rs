//! Compiled client against authenticated loopback fixtures, never real host tools.
use coeus::{
    common::{proto::*, stream::EncryptedListener},
    jobs::{DeploymentExecutor, JobContext},
    server::CoeusServer,
};
use eyre::Result;
use futures_util::future::BoxFuture;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
const KEY: [u8; 32] = [0x42; 32];
struct Fixture;
impl DeploymentExecutor for Fixture {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.phase(DeployPhase::Build).await?;
            context
                .log(LogSource::Stdout, "\x1b[32mfixture compiler output\x1b[0m")
                .await;
            if request.rev == "fail" {
                eyre::bail!("synthetic build failed");
            }
            if request.rev == "complete" {
                return Ok(());
            }
            context.cancelled().await;
            context.check_cancelled()
        })
    }
}
struct Harness {
    config: PathBuf,
    _dir: tempfile::TempDir,
    endpoint: String,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Harness {
    async fn new() -> Result<Self> {
        let server =
            CoeusServer::with_executor("127.0.0.1:0".parse()?, &KEY, Arc::new(Fixture)).await?;
        let endpoint = server.local_addr()?.to_string();
        let dir = tempfile::tempdir()?;
        let config = dir.path().join("config.toml");
        std::fs::write(
            &config,
            format!(
                "[settings]\nkey='{}'\nfleet=['{endpoint}']\ndefault_revision='complete'\n",
                "42".repeat(32)
            ),
        )?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(server.run_until_cancelled(shutdown.clone()));
        Ok(Self {
            config,
            _dir: dir,
            endpoint,
            shutdown,
            task,
        })
    }
    async fn stop(self) -> Result<()> {
        self.shutdown.cancel();
        self.task.await??;
        Ok(())
    }
}
async fn cli(config: &Path, args: &[&str]) -> Result<std::process::Output> {
    Ok(timeout(
        Duration::from_secs(5),
        Command::new(env!("CARGO_BIN_EXE_coeus"))
            .arg("--config")
            .arg(config)
            .args(args)
            .env("NO_COLOR", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await??)
}
fn success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn dispatch_status_logs_cancel_and_server_ids_are_target_aware() -> Result<()> {
    timeout(Duration::from_secs(15), async {
        let h = Harness::new().await?;
        let accepted = cli(
            &h.config,
            &[
                "deploy",
                &h.endpoint,
                "--rev",
                "hold",
                "--request-id",
                "701",
                "--no-tty",
            ],
        )
        .await?;
        success(&accepted);
        let output = String::from_utf8_lossy(&accepted.stdout);
        assert!(output.contains("request-id: 701"));
        assert!(output.contains("accepted 1"));
        assert!(output.find("request-id: 701") < output.find("accepted 1"));
        let retry = cli(
            &h.config,
            &[
                "deploy",
                &h.endpoint,
                "--rev",
                "hold",
                "--request-id",
                "701",
            ],
        )
        .await?;
        success(&retry);
        assert!(String::from_utf8_lossy(&retry.stdout).contains("accepted 1"));
        let status = cli(&h.config, &["status", &h.endpoint]).await?;
        success(&status);
        assert!(String::from_utf8_lossy(&status.stdout).contains("hold"));
        let fleet = cli(&h.config, &["status"]).await?;
        success(&fleet);
        assert!(!fleet.stdout.windows(2).any(|b| b == b"\x1b["));
        let logs = cli(&h.config, &["logs", "1", "--target", &h.endpoint]).await?;
        success(&logs);
        assert!(String::from_utf8_lossy(&logs.stdout).contains("fixture compiler output"));
        let cancelled = cli(&h.config, &["cancel", "1", "--target", &h.endpoint]).await?;
        success(&cancelled);
        let terminal = cli(&h.config, &["status", &h.endpoint, "--job", "1", "--watch"]).await?;
        success(&terminal);
        assert!(String::from_utf8_lossy(&terminal.stdout).contains("Cancelled"));
        let next = cli(&h.config, &["deploy"]).await?;
        success(&next);
        assert!(String::from_utf8_lossy(&next.stdout).contains("accepted 2"));
        let terminal = cli(&h.config, &["status", &h.endpoint, "--job", "2", "--watch"]).await?;
        success(&terminal);
        assert!(String::from_utf8_lossy(&terminal.stdout).contains("complete"));
        h.stop().await
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn fleet_dispatches_all_and_reports_partial_failures_without_retry() -> Result<()> {
    let a = Harness::new().await?;
    let b = Harness::new().await?;
    std::fs::write(
        &a.config,
        format!(
            "[settings]\nkey='{}'\nfleet=['{}','{}','127.0.0.1:1']\ndefault_revision='complete'\n",
            "42".repeat(32),
            a.endpoint,
            b.endpoint
        ),
    )?;
    let dispatched = cli(&a.config, &["deploy", "--no-tty"]).await?;
    assert!(!dispatched.status.success());
    let text = String::from_utf8_lossy(&dispatched.stdout);
    assert_eq!(text.matches("accepted 1").count(), 2);
    assert!(!text.contains("\x1b[?1049h"));
    for h in [&a, &b] {
        let status = cli(&a.config, &["status", &h.endpoint]).await?;
        success(&status);
        assert!(String::from_utf8_lossy(&status.stdout).contains("complete"));
    }
    a.stop().await?;
    b.stop().await?;
    Ok(())
}

#[tokio::test]
async fn wrong_key_never_becomes_bootstrap_and_missing_fleet_is_error() -> Result<()> {
    let h = Harness::new().await?;
    std::fs::write(
        &h.config,
        format!("[settings]\nkey='{}'\n", "43".repeat(32)),
    )?;
    let empty = cli(&h.config, &["deploy"]).await?;
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("no fleet"));
    let wrong = cli(&h.config, &["deploy", &h.endpoint]).await?;
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("bootstrap forbidden"));
    assert!(!String::from_utf8_lossy(&wrong.stdout).contains("request-id:"));
    h.stop().await
}

#[tokio::test]
async fn authenticated_bad_or_missing_hello_never_dispatches_or_offers_bootstrap() -> Result<()> {
    timeout(Duration::from_secs(15), async {
        for mode in ["wrong-version", "wrong-packet", "closed-before-hello"] {
            let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &KEY).await?;
            let endpoint = listener.local_addr()?.to_string();
            let dir = tempfile::tempdir()?;
            let config = dir.path().join("config");
            std::fs::write(
                &config,
                format!(
                    "[settings]\nkey='{}'\n[bootstrap.'{endpoint}']\nactivation_directory='/nonexistent/activation'\npeer_config='/nonexistent/peer'\nflake='github:example/config/{}'\nhostname='fixture'\nnix_binary='/nonexistent/nix'\n",
                    "42".repeat(32), "a".repeat(40)
                ),
            )?;
            let peer = tokio::spawn(async move {
                let (mut peer, _) = listener.accept().await?;
                assert!(matches!(peer.recv().await?, Some(Packet::HelloRequest(_))));
                match mode {
                    "wrong-version" => {
                        peer.send(HelloResponse {
                            protocol_version: PROTOCOL_VERSION + 1,
                            version: "fixture".into(),
                        }).await?;
                    }
                    "wrong-packet" => {
                        peer.send(StatusResponse { jobs: vec![], next_after: None }).await?;
                    }
                    _ => return Ok::<_, eyre::Report>(()),
                }
                // No deploy/nonce request is permitted after failed authenticated hello.
                assert!(peer.recv().await?.is_none());
                Ok(())
            });
            let result = cli(&config, &["deploy", &endpoint]).await?;
            assert!(!result.status.success(), "{mode}");
            let error = String::from_utf8_lossy(&result.stderr);
            assert!(error.contains("bootstrap forbidden"), "{mode}: {error}");
            assert!(!error.contains("[y/N]"), "{mode}: {error}");
            assert!(!String::from_utf8_lossy(&result.stdout).contains("request-id:"));
            peer.await??;
        }
        Ok::<_, eyre::Report>(())
    }).await??;
    Ok(())
}

#[tokio::test]
async fn quiet_retry_nonce_is_flushed_before_lost_ack_and_not_retried() -> Result<()> {
    timeout(Duration::from_secs(10), async {
        let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &KEY).await?;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config");
        std::fs::write(&path, format!("[settings]\nkey='{}'\n", "42".repeat(32)))?;
        let endpoint = listener.local_addr()?.to_string();
        let mut child = Command::new(env!("CARGO_BIN_EXE_coeus"))
            .arg("--config")
            .arg(&path)
            .args(["--verbosity", "off", "deploy", &endpoint])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let (mut peer, _) = listener.accept().await?;
        assert!(matches!(peer.recv().await?, Some(Packet::HelloRequest(_))));
        peer.send(HelloResponse {
            protocol_version: PROTOCOL_VERSION,
            version: "fixture".into(),
        })
        .await?;
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut label = String::new();
        out.read_line(&mut label).await?;
        assert!(label.contains("target:"));
        let mut nonce = String::new();
        out.read_line(&mut nonce).await?;
        assert!(nonce.contains("request-id:"));
        let Some(Packet::DeployRequest(request)) = peer.recv().await? else {
            panic!("expected request")
        };
        assert!(nonce.contains(&request.request_id.to_string()));
        drop(peer);
        let result = child.wait_with_output().await?;
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("query this target's status"));
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[test]
fn identity_needs_no_config_and_attach_takes_target_not_job_id() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("absent");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_coeus"))
        .args(["--config", absent.to_str().unwrap(), "identity"])
        .output()
        .unwrap();
    success(&output);
    assert!(!absent.exists());
    let identity: coeus::activation::BinaryIdentity =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(identity, coeus::activation::BinaryIdentity::current());
}

#[tokio::test]
async fn xdg_default_path_and_reserved_alias_work_without_host_settings() -> Result<()> {
    let h = Harness::new().await?;
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("coeus"))?;
    let port = h.endpoint.parse::<std::net::SocketAddr>()?.port();
    std::fs::write(
        dir.path().join("coeus/config.toml"),
        format!(
            "[settings]\nkey='{}'\nfleet=['me:{port}']\ndefault_revision='complete'\n",
            "42".repeat(32)
        ),
    )?;
    let output = timeout(
        Duration::from_secs(5),
        Command::new(env!("CARGO_BIN_EXE_coeus"))
            .args(["deploy", "--no-tty"])
            .env("XDG_CONFIG_HOME", dir.path())
            .env("HOME", dir.path().join("absent-home"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("accepted 1"));
    let status = cli(&h.config, &["status", &h.endpoint, "--job", "1", "--watch"]).await?;
    success(&status);
    assert!(String::from_utf8_lossy(&status.stdout).contains("complete"));
    h.stop().await
}

#[tokio::test]
async fn alias_duplicates_cannot_dispatch_two_jobs_to_same_socket() -> Result<()> {
    let h = Harness::new().await?;
    let port = h.endpoint.parse::<std::net::SocketAddr>()?.port();
    std::fs::write(
        &h.config,
        format!(
            "[settings]\nkey='{}'\nfleet=['me:{port}','localhost:{port}']\ndefault_revision='complete'\n",
            "42".repeat(32)
        ),
    )?;
    let output = cli(&h.config, &["deploy", "--no-tty"]).await?;
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .matches("accepted 1")
            .count(),
        1
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("duplicate fleet endpoint"));
    let status = cli(&h.config, &["status", &h.endpoint]).await?;
    success(&status);
    assert!(String::from_utf8_lossy(&status.stdout).contains("complete"));
    h.stop().await
}

#[cfg(unix)]
#[tokio::test]
async fn unreachable_tty_bootstrap_requires_explicit_yes_and_no_tty_never_prompts() -> Result<()> {
    use std::io::Write;
    timeout(Duration::from_secs(10),async {
        for no_tty in [false,true] {
            let dir=tempfile::tempdir()?;
            let listener=std::net::TcpListener::bind("127.0.0.1:0")?;
            let port=listener.local_addr()?.port(); drop(listener);
            let target=format!("me:{port}");
            let config=dir.path().join("config");
            std::fs::write(&config,format!("[settings]\nkey='{}'\n[bootstrap.'{target}']\nactivation_directory='/nonexistent/activation'\npeer_config='/nonexistent/peer'\nflake='github:example/config/{}'\nhostname='fixture'\nnix_binary='/nonexistent/nix'\n", "42".repeat(32),"a".repeat(40)))?;
            let pty=nix::pty::openpty(None,None)?;
            let mut master=std::fs::File::from(pty.master);let slave=std::fs::File::from(pty.slave);
            let mut command=Command::new(env!("CARGO_BIN_EXE_coeus"));
            command.arg("--config").arg(config).args(["deploy",&target]).stdin(slave.try_clone()?).stdout(slave).stderr(Stdio::piped()).kill_on_drop(true);
            if no_tty {command.arg("--no-tty");}
            let mut child=command.spawn()?;
            if !no_tty {
                let mut error=BufReader::new(child.stderr.take().unwrap());let mut prompt=String::new();
                error.read_line(&mut prompt).await?;
                assert!(prompt.contains("Bootstrap via verified SSH") && prompt.contains("[y/N]"));
                master.write_all(b"n\n")?;
                let status=child.wait().await?;assert!(!status.success());
            } else {
                let output=child.wait_with_output().await?;
                assert!(!output.status.success());
                let error=String::from_utf8_lossy(&output.stderr);
                assert!(!error.contains("[y/N]"));assert!(error.contains("unreachable"));
            }
        }
        Ok::<_,eyre::Report>(())
    }).await??;
    Ok(())
}
