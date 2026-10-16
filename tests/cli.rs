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
        assert!(output.contains("job 1 accepted"));
        assert!(output.find("request-id: 701") < output.find("job 1 accepted"));
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
        assert!(String::from_utf8_lossy(&retry.stdout).contains("job 1 accepted"));
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
        assert!(String::from_utf8_lossy(&next.stdout).contains("job 2 accepted"));
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
    assert_eq!(text.matches("job 1 accepted").count(), 2);
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
        assert!(String::from_utf8_lossy(&result.stderr).contains("Acceptance unknown"));
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("job 1 accepted"));
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
            .matches("job 1 accepted")
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

#[tokio::test]
async fn compiled_authenticated_refusal_keeps_acceptance_uncertain_without_retry() -> Result<()> {
    timeout(Duration::from_secs(5), async {
        let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &KEY).await?;
        let target = listener.local_addr()?.to_string();
        let dir = tempfile::tempdir()?;
        let config = dir.path().join("config.toml");
        std::fs::write(&config, format!("[settings]\nkey='{}'\n", "42".repeat(32)))?;
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            assert!(matches!(stream.recv().await?, Some(Packet::HelloRequest(_))));
            stream.send(HelloResponse { protocol_version: PROTOCOL_VERSION, version: "fixture".into() }).await?;
            assert!(matches!(stream.recv().await?, Some(Packet::DeployRequest(_))));
            stream.send(RequestRejected { message: "interrupted deployment requires operator reconciliation; new admission blocked".into() }).await?;
            assert!(stream.recv().await?.is_none(), "client must not resubmit on the same connection");
            assert!(timeout(Duration::from_millis(50), listener.accept()).await.is_err(), "client must not reconnect/retry");
            Ok::<_, eyre::Report>(())
        });
        let output = cli(&config, &["deploy", &target, "--no-tty"]).await?;
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("rejection") && error.contains("new admission blocked"));
        assert!(error.contains("Acceptance unknown"));
        assert!(!error.contains("Not submitted"));
        assert!(!error.contains("bootstrap") && !error.contains('—') && !error.contains('–'));
        peer.await??;
        Ok::<_, eyre::Report>(())
    }).await??;
    Ok(())
}

#[test]
fn operator_recovery_help_and_attestation_flags_are_local_and_config_explicit() -> Result<()> {
    use clap::Parser;
    use coeus::cli::main::{ClientCommand, MainCLIArgs};
    let args = MainCLIArgs::try_parse_from([
        "coeus",
        "recover",
        "1",
        "--server-config",
        "/fixture/server.toml",
    ])?;
    assert!(matches!(args.command, ClientCommand::Recover { .. }));
    assert!(
        MainCLIArgs::try_parse_from([
            "coeus",
            "recover",
            "1",
            "--server-config",
            "/fixture/server.toml",
            "--confirm",
            "digest"
        ])
        .is_err()
    );
    assert!(
        MainCLIArgs::try_parse_from([
            "coeus",
            "recover",
            "1",
            "--server-config",
            "/fixture/server.toml",
            "--attest-no-writers"
        ])
        .is_err()
    );
    assert!(
        MainCLIArgs::try_parse_from([
            "coeus",
            "recover",
            "1",
            "--server-config",
            "/fixture/server.toml",
            "--activation-directory",
            "/unrelated"
        ])
        .is_err()
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_coeus"))
        .args(["recover", "--help"])
        .output()?;
    success(&output);
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.contains("--server-config")
            && help.contains("--attest-no-writers")
            && help.contains("--accept-partial-state")
    );
    assert!(!help.contains("--ssh") && !help.contains("--activation-directory"));
    Ok(())
}

/// Actual compiled renderers against authenticated peers with controllable states.
/// No checkout, builder, service, privilege helper or host state is used here.
#[cfg(unix)]
mod display_acceptance {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::RwLock;

    struct Peer {
        address: String,
        job: Arc<RwLock<JobStatus>>,
        polls: Arc<AtomicUsize>,
        stop: CancellationToken,
        task: tokio::task::JoinHandle<Result<()>>,
    }
    impl Peer {
        async fn new(phase: DeployPhase, delay_ms: u64, ack: &'static str) -> Result<Self> {
            let listener = EncryptedListener::bind("127.0.0.1:0".parse()?, &KEY).await?;
            let address = listener.local_addr()?.to_string();
            let job = Arc::new(RwLock::new(JobStatus {
                job_id: u64::MAX,
                revision: "main".into(),
                commit: None,
                phase,
                cancel_requested: false,
                message: None,
            }));
            let polls = Arc::new(AtomicUsize::new(0));
            let stop = CancellationToken::new();
            let state = job.clone();
            let token = stop.clone();
            let count = polls.clone();
            let task = tokio::spawn(async move {
                let mut connections = tokio::task::JoinSet::new();
                loop {
                    let accepted = tokio::select! { biased; _ = token.cancelled() => break, result = listener.accept() => result? };
                    let (mut stream, _) = accepted;
                    let state = state.clone();
                    let count = count.clone();
                    connections.spawn(async move {
                        while let Some(packet) = stream.recv().await? {
                            match packet {
                                Packet::HelloRequest(_) => {
                                    stream
                                        .send(HelloResponse {
                                            protocol_version: PROTOCOL_VERSION,
                                            version: "fixture".into(),
                                        })
                                        .await?
                                }
                                Packet::DeployRequest(_) => match ack {
                                    "lost" => break,
                                    "zero" => {
                                        stream
                                            .send(DeployAccepted {
                                                job_id: 0,
                                                message: "invalid fixture ID".into(),
                                            })
                                            .await?
                                    }
                                    "wrong" => {
                                        stream
                                            .send(StatusResponse {
                                                jobs: vec![],
                                                next_after: None,
                                            })
                                            .await?
                                    }
                                    "persist" => {
                                        stream.send(RequestRejected { message: "persisting job record: parent directory fsync failed after atomic rename".into() }).await?
                                    }
                                    "reject" => {
                                        stream
                                            .send(RequestRejected {
                                                message: "new deployments blocked".into(),
                                            })
                                            .await?
                                    }
                                    _ => {
                                        stream
                                            .send(DeployAccepted {
                                                job_id: u64::MAX,
                                                message: "fixture accepted".into(),
                                            })
                                            .await?
                                    }
                                },
                                Packet::MachineStatusRequest(_) => {
                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                    count.fetch_add(1, Ordering::SeqCst);
                                    let job = state.read().await.clone();
                                    stream
                                        .send(MachineStatusResponse {
                                            active: Some(job.clone()),
                                            latest: Some(job),
                                        })
                                        .await?;
                                }
                                Packet::StatusRequest(_) => {
                                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                                    count.fetch_add(1, Ordering::SeqCst);
                                    stream
                                        .send(StatusResponse {
                                            jobs: vec![state.read().await.clone()],
                                            next_after: None,
                                        })
                                        .await?;
                                }
                                // Output-only observers must never send cancellation or input.
                                _ => {
                                    eyre::bail!("unexpected packet in display fixture");
                                }
                            }
                        }
                        Ok::<_, eyre::Report>(())
                    });
                }
                connections.abort_all();
                while connections.join_next().await.is_some() {}
                Ok(())
            });
            Ok(Self {
                address,
                job,
                polls,
                stop,
                task,
            })
        }
        async fn finish(self) -> Result<()> {
            self.stop.cancel();
            self.task.await?
        }
    }
    fn config(directory: &Path, peers: &[&Peer]) -> Result<PathBuf> {
        let path = directory.join("display.toml");
        let fleet = peers
            .iter()
            .map(|p| format!("'{}'", p.address))
            .collect::<Vec<_>>()
            .join(",");
        std::fs::write(
            &path,
            format!("[settings]\nkey='{}'\nfleet=[{fleet}]\n", "42".repeat(32)),
        )?;
        Ok(path)
    }
    fn command(config: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_coeus"));
        command
            .arg("--config")
            .arg(config)
            .args(args)
            .env("TERM", "xterm-256color")
            .env("CLICOLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .kill_on_drop(true);
        command
    }
    async fn polled(peer: &Peer, minimum: usize) {
        while peer.polls.load(Ordering::SeqCst) < minimum {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    fn assert_plain(text: &[u8]) {
        let text = String::from_utf8_lossy(text);
        assert!(!text.contains(['\x1b', '\t', '—', '–']), "{text}");
    }
    fn evidence(name: &str, bytes: &[u8]) -> Result<()> {
        if let Some(path) = std::env::var_os("COEUS_UX_EVIDENCE_DIR") {
            let path = PathBuf::from(path);
            std::fs::create_dir_all(&path)?;
            std::fs::write(path.join(name), bytes)?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn compiled_status_preserves_order_and_plain_watch_emits_only_changes() -> Result<()> {
        timeout(Duration::from_secs(12), async {
            let a = Peer::new(DeployPhase::Interrupted, 65, "accept").await?;
            let b = Peer::new(DeployPhase::Failed, 0, "accept").await?;
            let c = Peer::new(DeployPhase::RequiresManualSwitch, 5, "accept").await?;
            a.job.write().await.cancel_requested = true;
            b.job.write().await.message =
                Some("Operator attested partial state. Not verified.".into());
            let dir = tempfile::tempdir()?;
            let config = config(dir.path(), &[&a, &b, &c])?;
            let output = command(&config, &["status"])
                .env_remove("NO_COLOR")
                .output()
                .await?;
            success(&output);
            assert_plain(&output.stdout);
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(text.find(&a.address) < text.find(&b.address));
            assert!(text.find(&b.address) < text.find(&c.address));
            for expected in [
                "Recovery required",
                "Cancel requested.",
                "New deployments blocked.",
                "Failed",
                "Operator attested",
                "Waiting for switch",
                "18446744073709551615",
            ] {
                assert!(text.contains(expected), "{text}");
            }
            evidence("status-plain-states.txt", &output.stdout)?;
            let child = command(&config, &["status", "--watch"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            timeout(Duration::from_secs(3), polled(&a, 4))
                .await
                .expect("watch made no queries");
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(child.id().unwrap() as i32),
                nix::sys::signal::Signal::SIGINT,
            )?;
            let output = timeout(Duration::from_secs(2), child.wait_with_output())
                .await
                .expect("watch did not detach")?;
            success(&output);
            assert_plain(&output.stdout);
            let text = String::from_utf8_lossy(&output.stdout);
            assert_eq!(
                text.matches("Recovery required").count(),
                1,
                "unchanged polls must not flood pipes: {text}"
            );
            evidence("status-watch-changed-only.txt", &output.stdout)?;
            a.finish().await?;
            b.finish().await?;
            c.finish().await?;
            Ok::<_, eyre::Report>(())
        })
        .await??;
        Ok(())
    }

    struct View {
        master: std::fs::File,
        slave: std::fs::File,
        before: nix::sys::termios::Termios,
        child: tokio::process::Child,
        parser: vt100::Parser,
        bytes: Vec<u8>,
    }
    impl View {
        fn new(config: &Path, columns: u16, rows: u16, no_color: bool) -> Result<Self> {
            use nix::fcntl::{FcntlArg, OFlag, fcntl};
            let pty = nix::pty::openpty(
                Some(&nix::pty::Winsize {
                    ws_col: columns,
                    ws_row: rows,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                }),
                None,
            )?;
            fcntl(&pty.master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
            let master = std::fs::File::from(pty.master);
            let slave = std::fs::File::from(pty.slave);
            let before = nix::sys::termios::tcgetattr(&slave)?;
            let mut command = command(config, &["deploy"]);
            command
                .stdin(slave.try_clone()?)
                .stdout(slave.try_clone()?)
                .stderr(Stdio::piped());
            if no_color {
                command.env("NO_COLOR", "1");
            } else {
                command.env_remove("NO_COLOR");
            }
            let child = command.spawn()?;
            Ok(Self {
                master,
                slave,
                before,
                child,
                parser: vt100::Parser::new(rows, columns, 0),
                bytes: Vec::new(),
            })
        }
        fn drain(&mut self) -> Result<usize> {
            use std::io::Read;
            let mut count = 0;
            loop {
                let mut bytes = [0; 16384];
                match self.master.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => {
                        self.parser.process(&bytes[..n]);
                        self.bytes.extend_from_slice(&bytes[..n]);
                        count += n;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.into()),
                }
            }
            Ok(count)
        }
        async fn until(&mut self, text: &str) -> Result<()> {
            loop {
                self.drain()?;
                if self.parser.screen().contents().contains(text) {
                    return Ok(());
                }
                assert!(
                    self.child.try_wait()?.is_none(),
                    "view exited before {text}: {}",
                    String::from_utf8_lossy(&self.bytes)
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        fn resize(&mut self, columns: u16, rows: u16) -> Result<()> {
            rustix::termios::tcsetwinsize(
                &self.slave,
                rustix::termios::Winsize {
                    ws_col: columns,
                    ws_row: rows,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                },
            )?;
            self.parser.screen_mut().set_size(rows, columns);
            Ok(())
        }
        fn keys(&mut self, bytes: &[u8]) -> Result<()> {
            use std::io::Write;
            self.master.write_all(bytes)?;
            Ok(())
        }
        async fn exited(&mut self, succeeds: bool) -> Result<()> {
            loop {
                self.drain()?;
                if let Some(status) = self.child.try_wait()? {
                    assert_eq!(
                        status.success(),
                        succeeds,
                        "{}",
                        String::from_utf8_lossy(&self.bytes)
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            self.drain()?;
            assert_eq!(
                nix::sys::termios::tcgetattr(&self.slave)?,
                self.before,
                "restore all original terminal modes"
            );
            assert!(self.bytes.windows(8).any(|x| x == b"\x1b[?1049l"));
            Ok(())
        }
    }

    #[tokio::test]
    async fn compiled_fleet_tui_narrow_normal_wide_resize_scroll_color_and_restore() -> Result<()> {
        timeout(Duration::from_secs(20), async {
            for (columns, no_color) in [(32, false), (80, true), (120, false)] {
                let a = Peer::new(DeployPhase::Build, 45, "accept").await?;
                let b = Peer::new(DeployPhase::Build, 0, "accept").await?;
                let c = Peer::new(DeployPhase::RequiresManualSwitch, 10, "accept").await?;
                let d = Peer::new(DeployPhase::Build, 0, "accept").await?;
                let dir = tempfile::tempdir()?;
                let config = config(dir.path(), &[&a, &b, &c, &d])?;
                let mut view = View::new(&config, columns, 8, no_color)?;
                view.until("Building").await?;
                let screen = view.parser.screen().contents();
                assert!(screen.contains("Coeus fleet"));
                assert!(!screen.contains(['\t', '—', '–']));
                assert!(
                    !view.bytes.windows(4).any(|b| b == b"\x1b[2J"),
                    "no periodic full-screen clear"
                );
                if no_color {
                    assert!(!view.bytes.windows(5).any(|b| b == b"\x1b[36m"));
                } else {
                    assert!(
                        view.bytes.windows(5).any(|b| b == b"\x1b[36m"),
                        "active states have semantic cyan color"
                    );
                }
                evidence(
                    &format!(
                        "fleet-{columns}-{}.txt",
                        if no_color { "no-color" } else { "color" }
                    ),
                    screen.as_bytes(),
                )?;
                evidence(&format!("fleet-{columns}.ansi"), &view.bytes)?;
                if columns == 32 {
                    view.keys(b"G")?;
                    loop {
                        view.drain()?;
                        if view.parser.screen().contents() != screen {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    evidence(
                        "fleet-32-scrolled.txt",
                        view.parser.screen().contents().as_bytes(),
                    )?;
                    view.keys(b"g")?;
                }
                view.resize(120, 24)?;
                view.until("REVISION").await?;
                view.until(&d.address).await?;
                let wide = view.parser.screen().contents();
                assert!(wide.find(&a.address) < wide.find(&b.address));
                assert!(wide.find(&b.address) < wide.find(&c.address));
                assert!(wide.find(&c.address) < wide.find(&d.address));
                // Same statuses keep polling but must not repaint identical frames.
                polled(&a, 3).await;
                view.drain()?;
                tokio::time::sleep(Duration::from_millis(650)).await;
                assert_eq!(view.drain()?, 0, "unchanged status must not redraw");
                {
                    let mut job = a.job.write().await;
                    job.phase = DeployPhase::Interrupted;
                    job.cancel_requested = true;
                    job.message = Some("Activation result unknown.".into());
                }
                b.job.write().await.phase = DeployPhase::Cancelled;
                view.until("Recovery required").await?;
                view.until("Cancelled").await?;
                evidence(
                    &format!("fleet-{columns}-resized-states.txt"),
                    view.parser.screen().contents().as_bytes(),
                )?;
                view.keys(b"q")?;
                view.exited(true).await?;
                evidence(
                    &format!("fleet-{columns}-complete-transcript.ansi"),
                    &view.bytes,
                )?;
                a.finish().await?;
                b.finish().await?;
                c.finish().await?;
                d.finish().await?;
            }
            Ok::<_, eyre::Report>(())
        })
        .await??;
        Ok(())
    }

    #[tokio::test]
    async fn compiled_fleet_lost_ack_row_and_summary_never_claim_not_submitted() -> Result<()> {
        timeout(Duration::from_secs(15), async {
            for mode in ["lost", "zero", "wrong", "reject", "persist"] {
                let accepted = Peer::new(DeployPhase::Build, 0, "accept").await?;
                let failed = Peer::new(DeployPhase::Build, 0, mode).await?;
                let dir = tempfile::tempdir()?;
                let config = config(dir.path(), &[&accepted, &failed])?;
                let mut view = View::new(&config, 120, 24, true)?;
                let state = "Acceptance unknown";
                view.until(state).await?;
                let screen = view.parser.screen().contents();
                assert!(!screen.contains("Not submitted"));
                evidence(&format!("fleet-dispatch-{mode}.txt"), screen.as_bytes())?;
                accepted.job.write().await.phase = DeployPhase::Succeeded;
                view.exited(false).await?;
                let transcript = String::from_utf8_lossy(&view.bytes);
                assert!(!transcript.contains("Not submitted"));
                assert!(transcript.contains(state));
                accepted.finish().await?;
                failed.finish().await?;
            }
            Ok::<_, eyre::Report>(())
        })
        .await??;
        Ok(())
    }

    #[tokio::test]
    async fn compiled_dumb_terminal_never_enters_screen_or_emits_styles() -> Result<()> {
        timeout(Duration::from_secs(5), async {
            let peer = Peer::new(DeployPhase::Build, 0, "accept").await?;
            let dir = tempfile::tempdir()?;
            let config = config(dir.path(), &[&peer])?;
            use nix::fcntl::{FcntlArg, OFlag, fcntl};
            use std::io::Read;
            let pty = nix::pty::openpty(None, None)?;
            fcntl(&pty.master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
            let mut master = std::fs::File::from(pty.master);
            let slave = std::fs::File::from(pty.slave);
            let child = command(&config, &["deploy"])
                .env("TERM", "dumb")
                .env_remove("NO_COLOR")
                .stdin(slave.try_clone()?)
                .stdout(slave)
                .stderr(Stdio::piped())
                .spawn()?;
            let output = child.wait_with_output().await?;
            success(&output);
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                match master.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            || error.raw_os_error() == Some(5) =>
                    {
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            assert_plain(&bytes);
            assert_plain(&output.stderr);
            assert_eq!(
                peer.polls.load(Ordering::SeqCst),
                0,
                "dumb terminal dispatches only, no follow screen"
            );
            peer.finish().await
        })
        .await??;
        Ok(())
    }
}
