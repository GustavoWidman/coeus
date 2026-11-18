//! cloudflared, running only while the host is ready

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::{sync::watch, task::JoinHandle, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::{
    config::TunnelSettings,
    web::connector::{self, Command, Connector, Event, Timing},
};

const SECOND: Duration = Duration::from_secs(1);

/// feeds `events`, each `at` seconds after the start, and returns the
/// commands they drew, by second
fn drive(events: &[(u64, Event)]) -> (Connector, Vec<(u64, Command)>) {
    let start = Instant::now();
    let mut state = Connector::Stopped;
    let mut commands = Vec::new();
    for (at, event) in events {
        let (next, command) = state.on(*event, start + SECOND * *at as u32, &Timing::DEFAULT);
        state = next;
        commands.extend(command.map(|command| (*at, command)));
    }
    (state, commands)
}

#[test]
fn cloudflared_runs_from_rising_until_falling_and_is_killed_if_it_lingers() {
    use {Command::*, Event::*};

    let (state, commands) = drive(&[(0, Rose), (10, Fell), (24, Tick), (25, Tick), (25, Exited)]);
    assert_eq!(commands, [(0, Spawn), (10, Term), (25, Kill)]);
    assert_eq!(state, Connector::Stopped);

    let (state, commands) = drive(&[(0, Rose), (10, Fell), (12, Exited), (30, Tick)]);
    assert_eq!(commands, [(0, Spawn), (10, Term)], "it left in time");
    assert_eq!(state, Connector::Stopped);
}

#[test]
fn a_host_ready_again_while_cloudflared_leaves_starts_another_once_it_is_gone() {
    use {Command::*, Event::*};

    let (state, commands) = drive(&[(0, Rose), (10, Fell), (11, Rose), (12, Exited)]);
    assert_eq!(commands, [(0, Spawn), (10, Term), (12, Spawn)]);
    assert!(matches!(state, Connector::Running { .. }));

    let (state, commands) = drive(&[(0, Rose), (10, Fell), (11, Rose), (11, Fell), (12, Exited)]);
    assert_eq!(commands, [(0, Spawn), (10, Term)], "not if it fell again");
    assert_eq!(state, Connector::Stopped);
}

#[test]
fn a_cloudflared_that_dies_while_ready_restarts_after_1_2_5_then_10_s() {
    use Event::*;

    let start = Instant::now();
    let mut state = Connector::Stopped.on(Rose, start, &Timing::DEFAULT).0;
    let mut now = start;
    let mut waits = Vec::new();
    for _ in 0..6 {
        now += SECOND;
        state = state.on(Exited, now, &Timing::DEFAULT).0;
        let wake = state.wake().unwrap();
        assert_eq!(
            state.on(Tick, wake - Duration::from_millis(1), &Timing::DEFAULT),
            (state, None),
            "not before its time"
        );
        let (next, command) = state.on(Tick, wake, &Timing::DEFAULT);
        assert_eq!(command, Some(Command::Spawn));
        waits.push((wake - now).as_secs());
        (state, now) = (next, wake);
    }
    assert_eq!(waits, [1, 2, 5, 10, 10, 10]);

    now += Timing::DEFAULT.settled;
    state = state.on(Exited, now, &Timing::DEFAULT).0;
    assert_eq!(
        state.wake(),
        Some(now + SECOND),
        "one that ran a while dies as the first"
    );

    assert_eq!(
        state.on(Fell, now, &Timing::DEFAULT),
        (Connector::Stopped, None)
    );
}

#[test]
fn nothing_runs_while_the_host_is_not_ready() {
    use Event::*;

    let (state, commands) = drive(&[(0, Fell), (1, Tick), (2, Exited)]);
    assert_eq!((state, commands), (Connector::Stopped, vec![]));
}

/// a stand-in for cloudflared that logs its pid and arguments to `$LOG`,
/// then runs `then`
struct Stub {
    dir: tempfile::TempDir,
}

impl Stub {
    fn new(then: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("cloudflared");
        let script = format!(
            "#!/bin/sh\nLOG=\"$(dirname \"$0\")/log\"\necho \"start $$ $*\" >> \"$LOG\"\n{then}\n"
        );
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    /// logs "term" on SIGTERM and leaves
    fn polite() -> Self {
        Self::new("trap 'echo term >> \"$LOG\"; exit 0' TERM\nwhile :; do sleep 0.02; done")
    }

    /// logs "term" on SIGTERM and carries on
    fn stubborn() -> Self {
        Self::new("trap 'echo term >> \"$LOG\"' TERM\nwhile :; do sleep 0.02; done")
    }

    fn program(&self) -> PathBuf {
        self.dir.path().join("cloudflared")
    }

    fn lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// waits for the log to hold `count` lines
    async fn until(&self, count: usize) -> Vec<String> {
        tokio::time::timeout(5 * SECOND, async {
            loop {
                let lines = self.lines();
                if lines.len() >= count {
                    return lines;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the log holds only {:?}", self.lines()))
    }

    fn pid(line: &str) -> rustix::process::Pid {
        let pid = line.split(' ').nth(1).unwrap().parse().unwrap();
        rustix::process::Pid::from_raw(pid).unwrap()
    }
}

fn alive(pid: rustix::process::Pid) -> bool {
    rustix::process::test_kill_process(pid).is_ok()
}

const QUICK: Timing = Timing {
    grace: Duration::from_millis(200),
    kill_after: Duration::from_millis(400),
    restarts: [
        Duration::from_millis(100),
        Duration::from_millis(200),
        Duration::from_millis(400),
        Duration::from_millis(800),
    ],
    settled: Duration::from_secs(60),
};

struct Running {
    ready: watch::Sender<bool>,
    stop: CancellationToken,
    task: JoinHandle<()>,
}

fn supervise(program: PathBuf, config: &Path) -> Running {
    let (ready, receiver) = watch::channel(false);
    let stop = CancellationToken::new();
    let config = config.to_owned();
    let task = tokio::spawn({
        let stop = stop.clone();
        async move { connector::run(&program, &config, QUICK, receiver, stop).await }
    });
    Running { ready, stop, task }
}

#[tokio::test]
async fn cloudflared_starts_on_rising_with_its_config_and_leaves_on_sigterm() {
    let stub = Stub::polite();
    let running = supervise(stub.program(), Path::new("/run/coeus-web/cloudflared.json"));

    running.ready.send_replace(true);
    let lines = stub.until(1).await;
    let pid = Stub::pid(&lines[0]);
    let args = lines[0].splitn(3, ' ').nth(2).unwrap();
    assert_eq!(
        args,
        "tunnel --config /run/coeus-web/cloudflared.json --no-autoupdate --protocol quic \
         --grace-period 200ms run"
    );

    running.ready.send_replace(false);
    assert_eq!(stub.until(2).await[1], "term");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!alive(pid));

    running.stop.cancel();
    tokio::time::timeout(SECOND, running.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stub.lines().len(), 2, "nothing started again");
}

#[tokio::test]
async fn a_cloudflared_that_ignores_sigterm_is_killed_after_the_grace_period() {
    let stub = Stub::stubborn();
    let running = supervise(stub.program(), Path::new("config.json"));

    running.ready.send_replace(true);
    let pid = Stub::pid(&stub.until(1).await[0]);
    let fell = Instant::now();
    running.ready.send_replace(false);
    assert_eq!(stub.until(2).await[1], "term");

    while alive(pid) {
        assert!(
            fell.elapsed() < SECOND,
            "still alive after {:?}",
            fell.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        fell.elapsed() >= QUICK.kill_after,
        "killed after {:?}, before its grace period was up",
        fell.elapsed()
    );
}

#[tokio::test]
async fn a_cloudflared_that_keeps_dying_restarts_ever_more_slowly() {
    let stub = Stub::new("exit 1");
    let running = supervise(stub.program(), Path::new("config.json"));

    running.ready.send_replace(true);
    let mut starts = Vec::new();
    for count in 1..=4 {
        stub.until(count).await;
        starts.push(Instant::now());
    }
    let gaps: Vec<Duration> = starts.windows(2).map(|pair| pair[1] - pair[0]).collect();
    for (gap, wait) in gaps.iter().zip(QUICK.restarts) {
        assert!(
            *gap >= wait,
            "restarted after {gap:?}, sooner than {wait:?}"
        );
    }

    running.ready.send_replace(false);
    tokio::time::sleep(2 * QUICK.restarts[3]).await;
    let count = stub.lines().len();
    tokio::time::sleep(2 * QUICK.restarts[3]).await;
    assert_eq!(stub.lines().len(), count, "no restarts once not ready");
}

#[tokio::test]
async fn stopping_coeus_web_stops_cloudflared_and_waits_for_it() {
    let stub = Stub::polite();
    let running = supervise(stub.program(), Path::new("config.json"));

    running.ready.send_replace(true);
    let pid = Stub::pid(&stub.until(1).await[0]);
    running.stop.cancel();
    tokio::time::timeout(SECOND, running.task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stub.lines()[1], "term");
    assert!(!alive(pid));
}

#[tokio::test]
async fn a_cloudflared_that_cannot_start_is_tried_again() {
    let running = supervise(
        PathBuf::from("/nonexistent/cloudflared"),
        Path::new("c.json"),
    );
    running.ready.send_replace(true);
    tokio::time::sleep(Duration::from_millis(50)).await;
    running.ready.send_replace(false);
    running.stop.cancel();
    tokio::time::timeout(SECOND, running.task)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn the_config_points_the_tunnel_at_the_origin_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("origin.sock");
    let tunnel = TunnelSettings {
        id: "97355560-d822-4fcf-9fe7-e359a026ff85".into(),
        credentials_file: "/run/credentials/coeus-web.service/tunnel".into(),
        cloudflared: "cloudflared".into(),
    };
    let path = connector::configure(&tunnel, &socket).unwrap();
    assert_eq!(path, dir.path().join("cloudflared.json"));

    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        config,
        serde_json::json!({
            "tunnel": "97355560-d822-4fcf-9fe7-e359a026ff85",
            "credentials-file": "/run/credentials/coeus-web.service/tunnel",
            "ingress": [{ "service": format!("unix:{}", socket.display()) }],
        })
    );
}
