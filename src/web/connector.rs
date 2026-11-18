//! cloudflared, run as a child of `coeus web` only while the host is ready
//! ([super::ready]), so cloudflare sends visitors to hosts that can serve
//! them. cloudflared stays in `coeus web`'s process group, so the service
//! manager stops it with `coeus web` however that dies: systemd kills the
//! unit's control group, and launchd, unless `AbandonProcessGroup` is set,
//! the job's process group.

use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

use eyre::{Result, WrapErr, eyre};
use log::{error, info, warn};
use rustix::process::{Pid, Signal, kill_process};
use tokio::{
    process::{Child, Command as Process},
    sync::watch,
    time::{Instant, sleep_until},
};
use tokio_util::sync::CancellationToken;

use crate::config::TunnelSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// how long cloudflared finishes in-flight requests after SIGTERM
    pub grace: Duration,
    /// when SIGKILL follows SIGTERM
    pub kill_after: Duration,
    /// the waits before restarting a cloudflared that died while the host is
    /// ready, one more step for each death in a row
    pub restarts: [Duration; 4],
    /// how long cloudflared must run before its death counts as the first
    pub settled: Duration,
}

impl Timing {
    pub const DEFAULT: Self = Self {
        grace: Duration::from_secs(10),
        kill_after: Duration::from_secs(15),
        restarts: [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(5),
            Duration::from_secs(10),
        ],
        settled: Duration::from_secs(60),
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connector {
    Stopped,
    /// `deaths` is how many cloudflareds died in a row before this one
    Running {
        since: Instant,
        deaths: u8,
    },
    /// SIGTERM sent. SIGKILL follows at `kill_at`, unset once sent; another
    /// cloudflared starts after this one if the host is ready again
    Stopping {
        kill_at: Option<Instant>,
        respawn: bool,
    },
    /// cloudflared died while the host was ready; the next starts at `until`
    Backoff {
        until: Instant,
        deaths: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Rose,
    Fell,
    /// cloudflared exited, or could not start
    Exited,
    /// the time [Connector::wake] named has come
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Spawn,
    Term,
    Kill,
}

impl Connector {
    pub fn on(self, event: Event, now: Instant, timing: &Timing) -> (Self, Option<Command>) {
        use {Command::*, Connector::*, Event::*};

        let spawn = |deaths| (Running { since: now, deaths }, Some(Spawn));
        match (self, event) {
            (Stopped, Rose) => spawn(0),
            (Running { .. }, Fell) => (
                Stopping {
                    kill_at: Some(now + timing.kill_after),
                    respawn: false,
                },
                Some(Term),
            ),
            (Running { since, deaths }, Exited) => {
                let deaths = if now - since >= timing.settled {
                    0
                } else {
                    deaths
                };
                let wait = timing.restarts[usize::from(deaths).min(timing.restarts.len() - 1)];
                let deaths = deaths.saturating_add(1);
                (
                    Backoff {
                        until: now + wait,
                        deaths,
                    },
                    None,
                )
            }
            (Stopping { kill_at, .. }, Rose | Fell) => (
                Stopping {
                    kill_at,
                    respawn: event == Rose,
                },
                None,
            ),
            (Stopping { respawn: true, .. }, Exited) => spawn(0),
            (Stopping { respawn: false, .. }, Exited) => (Stopped, None),
            (
                Stopping {
                    kill_at: Some(at),
                    respawn,
                },
                Tick,
            ) if now >= at => (
                Stopping {
                    kill_at: None,
                    respawn,
                },
                Some(Kill),
            ),
            (Backoff { until, deaths }, Tick) if now >= until => spawn(deaths),
            (Backoff { .. }, Fell) => (Stopped, None),
            (state, _) => (state, None),
        }
    }

    /// when the connector acts next without being told, by a [Event::Tick]
    pub const fn wake(self) -> Option<Instant> {
        match self {
            Self::Stopping { kill_at, .. } => kill_at,
            Self::Backoff { until, .. } => Some(until),
            Self::Stopped | Self::Running { .. } => None,
        }
    }
}

/// writes cloudflared's configuration beside the origin socket: the tunnel,
/// its credentials, and every request to the socket. a `--url unix:` would
/// make cloudflared carry raw TCP over websockets, and `--unix-socket`
/// refuses a tunnel named on the command line, so this is the only way to
/// point a tunnel run by its credentials at a socket. YAML reads JSON.
pub fn configure(tunnel: &TunnelSettings, origin: &Path) -> Result<PathBuf> {
    let socket = origin
        .to_str()
        .ok_or_else(|| eyre!("{} is not UTF-8", origin.display()))?;
    let config = serde_json::json!({
        "tunnel": tunnel.id,
        "credentials-file": tunnel.credentials_file,
        "ingress": [{ "service": format!("unix:{socket}") }],
    });
    let path = origin.with_file_name("cloudflared.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&config)?)
        .wrap_err_with(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

/// runs cloudflared with `config` while `ready` holds true, and stops it
/// for good once `stop` is cancelled or readiness is no longer watched
pub async fn run(
    program: &Path,
    config: &Path,
    timing: Timing,
    mut ready: watch::Receiver<bool>,
    stop: CancellationToken,
) {
    let mut state = Connector::Stopped;
    let mut child: Option<Child> = None;
    let mut stopping = false;

    if *ready.borrow_and_update() {
        state = act(state, Event::Rose, &mut child, program, config, &timing);
    }

    while !(stopping && state == Connector::Stopped) {
        let wake = state.wake();
        let event = tokio::select! {
            () = stop.cancelled(), if !stopping => {
                stopping = true;
                Event::Fell
            }
            changed = ready.changed(), if !stopping => match changed {
                Ok(()) if *ready.borrow_and_update() => Event::Rose,
                Ok(()) => Event::Fell,
                Err(_) => {
                    stopping = true;
                    Event::Fell
                }
            },
            status = exit(&mut child) => {
                child = None;
                report(state, status);
                Event::Exited
            }
            () = sleep_until(wake.unwrap_or_else(Instant::now)), if wake.is_some() => Event::Tick,
        };
        state = act(state, event, &mut child, program, config, &timing);
    }
}

/// moves `state` on `event` and carries out what it commands
fn act(
    state: Connector,
    event: Event,
    child: &mut Option<Child>,
    program: &Path,
    config: &Path,
    timing: &Timing,
) -> Connector {
    let now = Instant::now();
    let (state, command) = state.on(event, now, timing);
    if let Connector::Backoff { until, .. } = state
        && event == Event::Exited
    {
        info!("starting cloudflared again in {:?}", until - now);
    }
    match command {
        Some(Command::Spawn) => match spawn(program, config, timing) {
            Ok(spawned) => {
                info!(
                    "connecting to the tunnel: cloudflared runs as pid {}",
                    spawned.id().unwrap_or_default()
                );
                *child = Some(spawned);
            }
            Err(error) => {
                error!("failed to start {}: {error}", program.display());
                return act(state, Event::Exited, child, program, config, timing);
            }
        },
        Some(Command::Term) => {
            info!("leaving the tunnel: cloudflared finishes its requests and exits");
            let pid = child
                .as_ref()
                .and_then(Child::id)
                .and_then(|id| Pid::from_raw(i32::try_from(id).ok()?));
            if let Some(pid) = pid {
                let _ = kill_process(pid, Signal::TERM);
            }
        }
        Some(Command::Kill) => {
            warn!("cloudflared outlived its grace period; killing it");
            if let Some(child) = child {
                let _ = child.start_kill();
            }
        }
        None => {}
    }
    state
}

fn spawn(program: &Path, config: &Path, timing: &Timing) -> std::io::Result<Child> {
    Process::new(program)
        .arg("tunnel")
        .arg("--config")
        .arg(config)
        .args(["--no-autoupdate", "--protocol", "quic", "--grace-period"])
        .arg(format!("{}ms", timing.grace.as_millis()))
        .arg("run")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

/// waits for `child` to exit, forever if there is none
async fn exit(child: &mut Option<Child>) -> std::io::Result<ExitStatus> {
    match child {
        Some(child) => child.wait().await,
        None => std::future::pending().await,
    }
}

fn report(state: Connector, status: std::io::Result<ExitStatus>) {
    let status = status.map_or_else(|error| error.to_string(), |status| status.to_string());
    match state {
        Connector::Running { .. } => warn!("cloudflared died while ready to serve: {status}"),
        _ => info!("cloudflared stopped: {status}"),
    }
}
