//! Genuine PTY + authenticated loopback + compiled CLI; never Deployer/nh.
#![cfg(unix)]
use coeus::{
    client::CoeusClient,
    common::proto::*,
    config::client_config,
    deploy::CommandRunner,
    jobs::{DeploymentExecutor, JobContext},
    server::CoeusServer,
};
use eyre::{Result, ensure};
use futures_util::future::BoxFuture;
use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::timeout,
};
use tokio_util::sync::CancellationToken;
const PSK: [u8; 32] = [0x64; 32];
struct Fixture {
    directory: PathBuf,
}
impl DeploymentExecutor for Fixture {
    fn execute(&self, request: DeployRequest, context: JobContext) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            context.phase(DeployPhase::Build).await?;
            let script = match request.rev.as_str() {
                "render" => {
                    r#"test -t 1 && test -t 2 && test ! -t 0 || exit 9
                    test "$TERM" = xterm-256color || exit 10
                    printf '\033[31mold\033[0m\r'; sleep 0.02
                    printf '\033[2K\033[3'; sleep 0.02; printf '2mcomplete'
                    printf '\360\237'; sleep 0.02; printf '\246\200\033[0m\n'
                    printf '\033]0;fixture'; sleep 0.02; printf '\007stderr\n' >&2"#
                }
                "detach" => {
                    r#"trap 'printf "unexpected SIGINT\n"; exit 19' INT
                    printf 'ready\n'; while [ ! -f release ]; do sleep 0.02; done; printf 'completed\n'"#
                }
                "geometry" => {
                    r#"stty size </dev/stdout; printf 'ready\n'; while [ ! -f release ]; do sleep 0.02; done; stty size </dev/stdout"#
                }
                "fail" => r#"printf '\033[31mfixture failed\033[0m\n' >&2; exit 23"#,
                "slow-view" => r#"head -c 100000 /dev/zero | tr '\000' x; printf 'completed\n'"#,
                "gap" => r#"head -c 300000 /dev/zero | tr '\000' x"#,
                _ => {
                    eyre::bail!("unknown fake request");
                }
            };
            CommandRunner::new("sh")
                .path(&self.directory)
                .args(["-c", script])
                .call_with_output(&context)
                .await
        })
    }
}
struct Harness {
    config: PathBuf,
    directory: tempfile::TempDir,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Harness {
    async fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let server = CoeusServer::with_durable_executor(
            "127.0.0.1:0".parse()?,
            &PSK,
            Arc::new(Fixture {
                directory: directory.path().into(),
            }),
            directory.path().join("ledger"),
            "fake-pty".into(),
        )
        .await?;
        let address = server.local_addr()?;
        let config = directory.path().join("client.toml");
        std::fs::write(
            &config,
            format!(
                "[settings]\nkey = \"{}\"\nhost = \"127.0.0.1\"\nport = {}\n",
                "64".repeat(32),
                address.port()
            ),
        )?;
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(server.run_until_cancelled(shutdown.clone()));
        Ok(Self {
            config,
            directory,
            shutdown,
            task,
        })
    }
    async fn client(&self) -> Result<CoeusClient> {
        CoeusClient::new(client_config(self.config.clone())?).await
    }
    async fn accept(&self, id: u64, rev: &str) -> Result<CoeusClient> {
        let mut client = self.client().await?;
        ensure!(
            matches!(
                client
                    .request(DeployRequest {
                        request_id: id,
                        rev: rev.into(),
                        dry_run: false,
                        clean_substituters: false
                    })
                    .await?,
                Packet::DeployAccepted(_)
            ),
            "missing ACK"
        );
        Ok(client)
    }
    async fn finish(&self, client: &mut CoeusClient, id: u64) -> Result<DeployPhase> {
        loop {
            let Packet::StatusResponse(status) = client
                .request(StatusRequest {
                    job_id: Some(id),
                    after: None,
                })
                .await?
            else {
                eyre::bail!("missing status");
            };
            if status.jobs[0].phase.is_terminal() {
                return Ok(status.jobs[0].phase);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_coeus"));
        command
            .arg("--config")
            .arg(&self.config)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }
    async fn stop(self) -> Result<()> {
        self.shutdown.cancel();
        self.task.await??;
        Ok(())
    }
}
async fn terminal(
    client: &mut CoeusClient,
    id: u64,
    after: Option<u64>,
    size: TerminalSize,
) -> Result<TerminalResponse> {
    let Packet::TerminalResponse(page) = client
        .request(TerminalRequest {
            job_id: id,
            after,
            size,
        })
        .await?
    else {
        eyre::bail!("missing terminal response");
    };
    Ok(*page)
}
async fn raw_history(client: &mut CoeusClient, id: u64) -> Result<Vec<u8>> {
    let mut cursor = 0;
    let mut raw = vec![];
    loop {
        let page = terminal(client, id, Some(cursor), Default::default()).await?;
        ensure!(page.available && !page.gap, "missing history");
        if page.bytes.is_empty() {
            break;
        }
        cursor = page.next_cursor;
        raw.extend(page.bytes);
    }
    Ok(raw)
}

#[tokio::test]
async fn compiled_attach_renders_color_redraw_unicode_and_merged_stderr_without_logger()
-> Result<()> {
    timeout(Duration::from_secs(12), async {
        let harness = Harness::new().await?;
        let mut client = harness.accept(801, "render").await?;
        assert_eq!(
            harness.finish(&mut client, 801).await?,
            DeployPhase::Succeeded
        );
        let raw = raw_history(&mut client, 801).await?;
        assert!(raw.starts_with(b"\x1b[31mold\x1b[0m\r"));
        let attached = harness
            .command(&["--verbosity", "trace", "attach", "801"])
            .output()
            .await?;
        assert!(
            attached.status.success(),
            "{}",
            String::from_utf8_lossy(&attached.stderr)
        );
        assert!(
            raw.windows(b"\x1b]0;fixture\x07".len())
                .any(|s| s == b"\x1b]0;fixture\x07")
        );
        assert_eq!(attached.stdout, raw); // No logger, UTF-8 replacement, prefixes or added newline.
        assert!(attached.stderr.is_empty());
        let mut screen = vt100::Parser::new(24, 80, 0);
        // Real terminal emulator, not ANSI stripping: split at every byte boundary.
        for byte in &attached.stdout {
            screen.process(&[*byte]);
        }
        assert_eq!(screen.screen().contents(), "complete🦀\nstderr");
        assert_eq!(
            screen.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Idx(2)
        );
        if let Some(directory) = std::env::var_os("COEUS_ATTACH_EVIDENCE_DIR") {
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory)?;
            std::fs::write(directory.join("attached.raw"), &attached.stdout)?;
            std::fs::write(
                directory.join("rendered-screen.txt"),
                screen.screen().contents(),
            )?;
            std::fs::write(
                directory.join("rendered-cell.txt"),
                format!(
                    "row 0 col 0: {:?}\n",
                    screen.screen().cell(0, 0).unwrap().fgcolor()
                ),
            )?;
        }
        let logs = harness
            .command(&["--verbosity", "off", "logs", "801"])
            .output()
            .await?;
        let compact = String::from_utf8(logs.stdout)?;
        assert!(compact.contains("complete🦀\nstderr\n"));
        assert!(
            !compact.contains("old")
                && !compact.contains('\x1b')
                && !compact.contains("[terminal]")
        );
        let metadata = harness
            .command(&[
                "--verbosity",
                "off",
                "logs",
                "801",
                "--metadata",
                "--source",
                "terminal",
            ])
            .output()
            .await?;
        assert!(String::from_utf8(metadata.stdout)?.contains("[terminal] complete🦀"));
        let stderr = harness
            .command(&["--verbosity", "off", "logs", "801", "--source", "stderr"])
            .output()
            .await?;
        assert!(stderr.stdout.is_empty()); // Never pretend merged PTY bytes are stderr.
        harness.accept(808, "fail").await?;
        assert_eq!(harness.finish(&mut client, 808).await?, DeployPhase::Failed);
        let failed = harness.command(&["attach", "808"]).output().await?;
        assert!(!failed.status.success());
        assert_eq!(failed.stdout, b"\x1b[31mfixture failed\x1b[0m\r\n");
        assert!(
            String::from_utf8_lossy(&failed.stderr).contains("deployment ended: command failed")
        );
        harness.stop().await
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn ctrl_c_detaches_compiled_client_and_job_completes_then_reattaches() -> Result<()> {
    timeout(Duration::from_secs(12), async {
        let harness = Harness::new().await?;
        let mut client = harness.accept(802, "detach").await?;
        let mut view = harness.command(&["attach", "802"]).spawn()?;
        let mut stdout = BufReader::new(view.stdout.take().unwrap());
        let mut ready = String::new();
        stdout.read_line(&mut ready).await?;
        assert_eq!(ready, "ready\r\n");
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(view.id().unwrap() as i32),
            nix::sys::signal::Signal::SIGINT,
        )?;
        assert!(view.wait().await?.success());
        let Packet::StatusResponse(status) = client
            .request(StatusRequest {
                job_id: Some(802),
                after: None,
            })
            .await?
        else {
            panic!("missing status");
        };
        assert_eq!(status.jobs[0].phase, DeployPhase::Build);
        assert!(!status.jobs[0].cancel_requested);
        // A separate connection can detach too without touching the executor.
        let connection = harness.client().await?;
        drop(connection);
        std::fs::write(harness.directory.path().join("release"), [])?;
        assert_eq!(
            harness.finish(&mut client, 802).await?,
            DeployPhase::Succeeded
        );
        let replay = harness.command(&["attach", "802"]).output().await?;
        assert_eq!(replay.stdout, b"ready\r\ncompleted\r\n");
        assert!(!String::from_utf8_lossy(&replay.stdout).contains("SIGINT"));
        harness.stop().await
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn resize_and_gap_are_explicit_and_cancel_remains_separate() -> Result<()> {
    timeout(Duration::from_secs(12), async {
        let harness = Harness::new().await?;
        let mut client = harness.accept(803, "geometry").await?;
        // Wait for the initial/default geometry output before resizing.
        loop {
            let page = terminal(&mut client, 803, Some(0), Default::default()).await?;
            if page.bytes.windows(5).any(|b| b == b"ready") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        terminal(
            &mut client,
            803,
            None,
            TerminalSize {
                rows: 40,
                cols: 120,
            },
        )
        .await?;
        std::fs::write(harness.directory.path().join("release"), [])?;
        assert_eq!(
            harness.finish(&mut client, 803).await?,
            DeployPhase::Succeeded
        );
        let raw = raw_history(&mut client, 803).await?;
        assert!(raw.starts_with(b"24 80\r\n"));
        assert!(raw.ends_with(b"40 120\r\n"));
        harness.accept(804, "gap").await?;
        assert_eq!(
            harness.finish(&mut client, 804).await?,
            DeployPhase::Succeeded
        );
        let gap = terminal(&mut client, 804, Some(0), Default::default()).await?;
        assert!(gap.gap && gap.bytes.is_empty());
        let attached = harness.command(&["attach", "804"]).output().await?;
        assert!(!attached.status.success() && attached.stdout.is_empty());
        assert!(String::from_utf8_lossy(&attached.stderr).contains("raw terminal history gap"));
        let live = harness
            .command(&["attach", "804", "--live"])
            .output()
            .await?;
        assert!(live.status.success() && live.stdout.is_empty());
        std::fs::remove_file(harness.directory.path().join("release"))?;
        harness.accept(805, "detach").await?;
        client.request(CancelRequest { job_id: 805 }).await?;
        assert_eq!(
            harness.finish(&mut client, 805).await?,
            DeployPhase::Cancelled
        );
        harness.stop().await
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn tty_ctrl_c_restores_local_modes_and_cursor_without_signalling_job() -> Result<()> {
    use std::io::Read;
    timeout(Duration::from_secs(12), async {
        let harness = Harness::new().await?;
        let mut client = harness.accept(806, "detach").await?;
        let pair = nix::pty::openpty(None, None)?;
        nix::fcntl::fcntl(
            &pair.master,
            nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
        )?;
        let mut master = std::fs::File::from(pair.master);
        let slave = std::fs::File::from(pair.slave);
        let before = nix::sys::termios::tcgetattr(&slave)?;
        let flags = nix::fcntl::fcntl(&slave, nix::fcntl::FcntlArg::F_GETFL)?;
        let mut command = harness.command(&["attach", "806"]);
        command.stdin(slave.try_clone()?).stdout(slave.try_clone()?);
        let mut view = command.spawn()?;
        let mut received = vec![];
        loop {
            let mut bytes = [0; 1024];
            match master.read(&mut bytes) {
                Ok(count) => received.extend(&bytes[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            if received.windows(5).any(|s| s == b"ready") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(view.id().unwrap() as i32),
            nix::sys::signal::Signal::SIGINT,
        )?;
        assert!(view.wait().await?.success());
        assert_eq!(nix::sys::termios::tcgetattr(&slave)?, before);
        assert_eq!(
            nix::fcntl::fcntl(&slave, nix::fcntl::FcntlArg::F_GETFL)?,
            flags
        );
        loop {
            let mut bytes = [0; 1024];
            match master.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => received.extend(&bytes[..count]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        assert!(received.ends_with(b"\x1b[0m\x1b[?25h"));
        std::fs::write(harness.directory.path().join("release"), [])?;
        assert_eq!(
            harness.finish(&mut client, 806).await?,
            DeployPhase::Succeeded
        );
        let raw = raw_history(&mut client, 806).await?;
        assert_eq!(raw, b"ready\r\ncompleted\r\n");
        harness.stop().await
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn backpressured_attach_is_cancellable_and_never_blocks_deployment() -> Result<()> {
    timeout(Duration::from_secs(12), async {
        let harness = Harness::new().await?;
        let mut client = harness.accept(807, "slow-view").await?;
        let mut view = harness.command(&["attach", "807"]).spawn()?;
        // Deliberately don't drain client's stdout pipe. Output exceeds pipe
        // capacity but remains within replay retention; the daemon still finishes.
        assert_eq!(
            harness.finish(&mut client, 807).await?,
            DeployPhase::Succeeded
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(view.try_wait()?.is_none());
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(view.id().unwrap() as i32),
            nix::sys::signal::Signal::SIGINT,
        )?;
        assert!(
            timeout(Duration::from_secs(2), view.wait())
                .await??
                .success()
        );
        let raw = raw_history(&mut client, 807).await?;
        assert_eq!(raw.len(), 100011);
        assert!(raw.ends_with(b"completed\r\n"));
        harness.stop().await
    })
    .await??;
    Ok(())
}
