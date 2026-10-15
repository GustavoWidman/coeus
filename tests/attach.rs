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
                "[settings]\nkey = \"{}\"\nfleet = [\"127.0.0.1:{}\"]\n",
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
        let config = client_config(self.config.clone())?;
        CoeusClient::new(config.fleet[0].parse()?, &config.key).await
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
            .args(
                if let Some(pos) = args.iter().position(|s| *s == "attach") {
                    let config = client_config(self.config.clone()).unwrap();
                    let mut args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
                    args[pos + 1] = config.fleet[0].clone();
                    args
                } else {
                    args.iter().map(|s| s.to_string()).collect()
                },
            )
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

async fn capture_attachment(mut command: Command, count: usize) -> Result<std::process::Output> {
    use tokio::io::AsyncReadExt;
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().unwrap();
    let mut bytes = vec![0; count];
    stdout.read_exact(&mut bytes).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id().unwrap() as i32),
        nix::sys::signal::Signal::SIGINT,
    )?;
    let mut result = child.wait_with_output().await?;
    stdout.read_to_end(&mut bytes).await?;
    result.stdout = bytes;
    Ok(result)
}

#[tokio::test]
async fn compiled_attach_renders_color_redraw_unicode_and_merged_stderr_without_logger()
-> Result<()> {
    timeout(Duration::from_secs(12), async {
        let harness = Harness::new().await?;
        let mut client = harness.accept(1, "render").await?;
        assert_eq!(
            harness.finish(&mut client, 1).await?,
            DeployPhase::Succeeded
        );
        let raw = raw_history(&mut client, 1).await?;
        assert!(raw.starts_with(b"\x1b[31mold\x1b[0m\r"));
        let attached = capture_attachment(
            harness.command(&["--verbosity", "trace", "attach", "1"]),
            raw.len(),
        )
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
            .command(&["--verbosity", "off", "logs", "1"])
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
                "1",
                "--metadata",
                "--source",
                "terminal",
            ])
            .output()
            .await?;
        assert!(String::from_utf8(metadata.stdout)?.contains("[terminal] complete🦀"));
        let stderr = harness
            .command(&["--verbosity", "off", "logs", "1", "--source", "stderr"])
            .output()
            .await?;
        assert!(stderr.stdout.is_empty()); // Never pretend merged PTY bytes are stderr.
        harness.accept(2, "fail").await?;
        assert_eq!(harness.finish(&mut client, 2).await?, DeployPhase::Failed);
        assert_eq!(
            raw_history(&mut client, 2).await?,
            b"\x1b[31mfixture failed\x1b[0m\r\n"
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
        let mut client = harness.accept(1, "detach").await?;
        let mut view = harness.command(&["attach", "1"]).spawn()?;
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
                job_id: Some(1),
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
            harness.finish(&mut client, 1).await?,
            DeployPhase::Succeeded
        );
        let replay = capture_attachment(
            harness.command(&["attach", "1"]),
            b"ready\r\ncompleted\r\n".len(),
        )
        .await?;
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
        let mut client = harness.accept(1, "geometry").await?;
        // Wait for the initial/default geometry output before resizing.
        loop {
            let page = terminal(&mut client, 1, Some(0), Default::default()).await?;
            if page.bytes.windows(5).any(|b| b == b"ready") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        terminal(
            &mut client,
            1,
            None,
            TerminalSize {
                rows: 40,
                cols: 120,
            },
        )
        .await?;
        std::fs::write(harness.directory.path().join("release"), [])?;
        assert_eq!(
            harness.finish(&mut client, 1).await?,
            DeployPhase::Succeeded
        );
        let raw = raw_history(&mut client, 1).await?;
        assert!(raw.starts_with(b"24 80\r\n"));
        assert!(raw.ends_with(b"40 120\r\n"));
        harness.accept(2, "gap").await?;
        assert_eq!(
            harness.finish(&mut client, 2).await?,
            DeployPhase::Succeeded
        );
        let gap = terminal(&mut client, 2, Some(0), Default::default()).await?;
        assert!(gap.gap && gap.bytes.is_empty());
        let attached = harness.command(&["attach", "2"]).output().await?;
        assert!(!attached.status.success() && attached.stdout.is_empty());
        assert!(String::from_utf8_lossy(&attached.stderr).contains("shared terminal history gap"));
        let live = capture_attachment(harness.command(&["attach", "2", "--live"]), 0).await?;
        assert!(live.status.success() && live.stdout.is_empty());
        std::fs::remove_file(harness.directory.path().join("release"))?;
        harness.accept(3, "detach").await?;
        client.request(CancelRequest { job_id: 3 }).await?;
        assert_eq!(
            harness.finish(&mut client, 3).await?,
            DeployPhase::Cancelled
        );
        harness.stop().await
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn tty_ctrl_c_restores_local_modes_and_cursor_without_signalling_job() -> Result<()> {
    check_tty_detach_restores_modes(false).await
}

#[tokio::test]
async fn tty_ctrl_c_preserves_preexisting_nonblocking_output() -> Result<()> {
    check_tty_detach_restores_modes(true).await
}

async fn check_tty_detach_restores_modes(initially_nonblocking: bool) -> Result<()> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use std::io::Read;
    timeout(Duration::from_secs(12), async {
        let harness = Harness::new().await?;
        let mut client = harness.accept(1, "detach").await?;
        let pair = nix::pty::openpty(None, None)?;
        nix::fcntl::fcntl(
            &pair.master,
            nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK),
        )?;
        let mut master = std::fs::File::from(pair.master);
        let slave = std::fs::File::from(pair.slave);
        let before = nix::sys::termios::tcgetattr(&slave)?;
        let mut initial_flags = OFlag::from_bits_truncate(fcntl(&slave, FcntlArg::F_GETFL)?);
        initial_flags.set(OFlag::O_NONBLOCK, initially_nonblocking);
        fcntl(&slave, FcntlArg::F_SETFL(initial_flags))?;
        // Compare access mode and meaningful F_SETFL status flags, not every
        // raw kernel bit. On Darwin an ordinary PTY write adds 0x10000 even
        // when F_SETFL restores the original flags. That write-history bit is
        // not an attach mode leak. In particular, O_NONBLOCK must be restored
        // in BOTH directions: initially blocking and initially nonblocking.
        let status_flags =
            OFlag::O_ACCMODE | OFlag::O_APPEND | OFlag::O_ASYNC | OFlag::O_NONBLOCK | OFlag::O_SYNC;
        let flags = OFlag::from_bits_truncate(fcntl(&slave, FcntlArg::F_GETFL)?) & status_flags;
        assert_eq!(flags.contains(OFlag::O_NONBLOCK), initially_nonblocking);
        let mut command = harness.command(&["attach", "1"]);
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
            OFlag::from_bits_truncate(fcntl(&slave, FcntlArg::F_GETFL)?) & status_flags,
            flags,
            "attach must restore output status flags after Ctrl+C"
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
            harness.finish(&mut client, 1).await?,
            DeployPhase::Succeeded
        );
        let raw = raw_history(&mut client, 1).await?;
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
        let mut client = harness.accept(1, "slow-view").await?;
        let mut view = harness.command(&["attach", "1"]).spawn()?;
        // Deliberately don't drain client's stdout pipe. Output exceeds pipe
        // capacity but remains within replay retention; the daemon still finishes.
        assert_eq!(
            harness.finish(&mut client, 1).await?,
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
        let raw = raw_history(&mut client, 1).await?;
        assert_eq!(raw.len(), 100011);
        assert!(raw.ends_with(b"completed\r\n"));
        harness.stop().await
    })
    .await??;
    Ok(())
}

/// A real terminal selects automatic single-job follow versus fleet TUI.
#[tokio::test]
async fn interactive_single_follows_terminal_and_fleet_restores_screen() -> Result<()> {
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use std::io::Read;
    timeout(Duration::from_secs(15), async {
        for (fleet, revision, succeeds) in [
            (false, "render", true),
            (true, "render", true),
            (false, "fail", false),
            (true, "fail", false),
        ] {
            let h = Harness::new().await?;
            let config = client_config(h.config.clone())?;
            let pair = nix::pty::openpty(None, None)?;
            fcntl(&pair.master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
            let mut master = std::fs::File::from(pair.master);
            let slave = std::fs::File::from(pair.slave);
            let before = nix::sys::termios::tcgetattr(&slave)?;
            let mut command = if fleet {
                h.command(&["deploy", "--rev", revision])
            } else {
                h.command(&["deploy", &config.fleet[0], "--rev", revision])
            };
            command.stdin(slave.try_clone()?).stdout(slave.try_clone()?);
            let mut child = command.spawn()?;
            let mut bytes = Vec::new();
            let status = loop {
                let mut buf = [0; 8192];
                loop {
                    match master.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => bytes.extend_from_slice(&buf[..n]),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) => return Err(e.into()),
                    }
                }
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            };
            // Drain bytes written just before exit.
            loop {
                let mut buf = [0; 8192];
                match master.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => bytes.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.into()),
                }
            }
            assert_eq!(
                status.success(),
                succeeds,
                "{}",
                String::from_utf8_lossy(&bytes)
            );
            assert_eq!(nix::sys::termios::tcgetattr(&slave)?, before);
            if fleet {
                assert!(bytes.windows(8).any(|s| s == b"\x1b[?1049h"));
                assert!(bytes.windows(8).any(|s| s == b"\x1b[?1049l"));
                assert!(String::from_utf8_lossy(&bytes).contains(if succeeds {
                    "Succeeded"
                } else {
                    "Failed"
                }));
            } else {
                assert!(!bytes.windows(8).any(|s| s == b"\x1b[?1049h"));
                assert!(String::from_utf8_lossy(&bytes).contains(if succeeds {
                    "complete"
                } else {
                    "fixture failed"
                }));
            }
            h.stop().await?;
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn two_compiled_subscribers_follow_shared_output_across_jobs() -> Result<()> {
    use tokio::io::AsyncReadExt;
    timeout(Duration::from_secs(12), async {
        let h = Harness::new().await?;
        let mut first = h.command(&["attach", "unused"]).spawn()?;
        let mut second = h.command(&["attach", "unused"]).spawn()?;
        let mut a = first.stdout.take().unwrap();
        let mut b = second.stdout.take().unwrap();
        for id in [1, 2] {
            let mut client = h.accept(id + 100, "render").await?;
            assert_eq!(h.finish(&mut client, id).await?, DeployPhase::Succeeded);
            let expected = raw_history(&mut client, id).await?;
            let mut one = vec![0; expected.len()];
            let mut two = vec![0; expected.len()];
            tokio::try_join!(a.read_exact(&mut one), b.read_exact(&mut two))?;
            assert_eq!(one, expected);
            assert_eq!(two, expected);
            assert!(first.try_wait()?.is_none() && second.try_wait()?.is_none());
        }
        for child in [&mut first, &mut second] {
            nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(child.id().unwrap() as i32),
                nix::sys::signal::Signal::SIGINT,
            )?;
            assert!(child.wait().await?.success());
        }
        h.stop().await
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn fleet_tui_restores_terminal_on_ctrl_c_and_disconnect_without_cancel_packet() -> Result<()>
{
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use std::io::Read;
    timeout(Duration::from_secs(15), async {
        for disconnect in [false, true] {
            let h = Harness::new().await?;
            let pair = nix::pty::openpty(None, None)?;
            fcntl(&pair.master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
            let mut master = std::fs::File::from(pair.master);
            let slave = std::fs::File::from(pair.slave);
            let before = nix::sys::termios::tcgetattr(&slave)?;
            let mut command = h.command(&["deploy", "--rev", "detach"]);
            command.stdin(slave.try_clone()?).stdout(slave.try_clone()?);
            let mut child = command.spawn()?;
            let mut received = Vec::new();
            loop {
                let mut buf = [0; 8192];
                match master.read(&mut buf) {
                    Ok(n) => received.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.into()),
                }
                if received.windows(8).any(|s| s == b"\x1b[?1049h") {
                    break;
                }
                assert!(child.try_wait()?.is_none());
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if disconnect {
                h.shutdown.cancel();
            } else {
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(child.id().unwrap() as i32),
                    nix::sys::signal::Signal::SIGINT,
                )?;
            }
            let status = child.wait().await?;
            assert_eq!(status.success(), !disconnect);
            loop {
                let mut buf = [0; 8192];
                match master.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => received.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.into()),
                }
            }
            assert!(received.windows(8).any(|s| s == b"\x1b[?1049l"));
            assert_eq!(nix::sys::termios::tcgetattr(&slave)?, before);
            if !disconnect {
                let mut client = h.client().await?;
                let Packet::StatusResponse(status) = client
                    .request(StatusRequest {
                        job_id: Some(1),
                        after: None,
                    })
                    .await?
                else {
                    eyre::bail!("missing status");
                };
                assert!(!status.jobs[0].cancel_requested);
                assert!(!status.jobs[0].phase.is_terminal());
                std::fs::write(h.directory.path().join("release"), [])?;
                assert_eq!(h.finish(&mut client, 1).await?, DeployPhase::Succeeded);
            }
            h.stop().await?;
        }
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn interactive_single_observed_cancellation_exits_error_and_restores_terminal() -> Result<()>
{
    use nix::fcntl::{FcntlArg, OFlag, fcntl};
    use std::io::Read;
    timeout(Duration::from_secs(10), async {
        let h = Harness::new().await?;
        let config = client_config(h.config.clone())?;
        let pty = nix::pty::openpty(None, None)?;
        fcntl(&pty.master, FcntlArg::F_SETFL(OFlag::O_NONBLOCK))?;
        let mut master = std::fs::File::from(pty.master);
        let slave = std::fs::File::from(pty.slave);
        let before = nix::sys::termios::tcgetattr(&slave)?;
        let mut command = h.command(&["deploy", &config.fleet[0], "--rev", "detach"]);
        command.stdin(slave.try_clone()?).stdout(slave.try_clone()?);
        let mut child = command.spawn()?;
        let mut received = Vec::new();
        loop {
            let mut b = [0; 8192];
            match master.read(&mut b) {
                Ok(n) => received.extend_from_slice(&b[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
            if received.windows(5).any(|s| s == b"ready") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut client = h.client().await?;
        client.request(CancelRequest { job_id: 1 }).await?;
        assert!(!child.wait().await?.success());
        assert_eq!(nix::sys::termios::tcgetattr(&slave)?, before);
        assert_eq!(h.finish(&mut client, 1).await?, DeployPhase::Cancelled);
        h.stop().await
    })
    .await??;
    Ok(())
}
