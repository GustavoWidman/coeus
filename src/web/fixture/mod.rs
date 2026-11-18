//! fixture mode: the real site over a simulated fleet whose clock can be
//! moved to any moment, so the web app can be built and tested with no fleet
//! at all. the simulation speaks to the aggregator as daemons do, with views
//! and links, and serves job logs as frames, so everything past it is the
//! code the real fleet runs through.

mod log;
mod story;
mod world;

use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::post,
};
use eyre::{Result, eyre};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use self::world::{HOSTS, STEP, World};
use super::{
    fleet::Input,
    session::Sessions,
    signin::Signin,
    site::{Access, Settings, Site},
    source::Source,
    view::Viewer,
    whois::{Peer, Policy, Whois},
};
use crate::{
    client::{AttachmentEvent, FleetEvent, Link, Offline, Unreach},
    common::model::{Hostname, JobState, MachineView, Timestamp},
    config::Key,
};

/// a simulated fleet
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Scenario {
    /// the dashboard's design prototype: six machines, a deployment rolling
    /// out in waves, a failure, a laptop asleep that wakes to catch up
    Instrument,
}

/// the moment the timeline starts: 2026-11-17 15:00 UTC
pub const START: i64 = 1_794_927_600_000;

pub struct Fixture {
    clock: Mutex<Clock>,
    /// one move reaches the aggregator whole before the next starts
    moving: tokio::sync::Mutex<()>,
    /// log readers look again after every move
    moved: watch::Sender<()>,
    inputs: OnceLock<mpsc::Sender<Input>>,
}

struct Clock {
    world: World,
    /// how many times the story started over before the world's
    turn: i64,
    /// how long the story runs before it starts over, in milliseconds
    period: i64,
    /// where the timeline is, in milliseconds after [START]
    at: f64,
    /// timeline milliseconds per real millisecond; 0 holds the clock
    speed: f64,
    /// what each machine last told the aggregator
    told: Vec<Option<(bool, MachineView)>>,
}

/// where the timeline is
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Status {
    /// seconds after the timeline starts
    pub t: f64,
    pub speed: f64,
    /// the moment it is in the simulated fleet, in epoch milliseconds
    pub now: i64,
    /// seconds before the story starts over
    pub period: f64,
}

impl Fixture {
    /// how often the clock moves while it runs
    const TICK: Duration = Duration::from_millis(100);

    pub fn new(scenario: Scenario, speed: f64) -> Arc<Self> {
        let Scenario::Instrument = scenario;
        let mut world = World::seed(START);
        let period = loop {
            world.step();
            if let Some(ended) = world.ended {
                break ended - START;
            }
        };

        Arc::new(Self {
            clock: Mutex::new(Clock {
                world: World::seed(START),
                turn: 0,
                period,
                at: 0.0,
                speed,
                told: vec![None; HOSTS.len()],
            }),
            moving: tokio::sync::Mutex::new(()),
            moved: watch::Sender::new(()),
            inputs: OnceLock::new(),
        })
    }

    pub fn hosts(&self) -> Vec<Hostname> {
        HOSTS.iter().map(|host| host.parse().unwrap()).collect()
    }

    /// who reads the simulated fleet
    pub fn viewer() -> Viewer {
        Viewer {
            login: "gustavo@github".into(),
            device: "iphone".into(),
            expires_at: Timestamp(START + 7 * 24 * 60 * 60 * 1000),
        }
    }

    /// the whole simulated site, served at `origin`: the fleet, its clock,
    /// and, unless `open`, signing in on the same address as
    /// [Fixture::viewer], with a session key made up for this run
    pub fn site(
        self: &Arc<Self>,
        web_dir: Option<PathBuf>,
        origin: &str,
        open: bool,
    ) -> Result<Router> {
        let viewer = Self::viewer();
        let sessions = Arc::new(Sessions::new(vec![Key::generate()], 7)?);
        let access = match open {
            true => Access::Open(viewer.clone()),
            false => Access::Sessions {
                sessions: sessions.clone(),
                signin: vec![origin.to_owned()],
            },
        };
        let source = Source::Fixture(self.clone());
        let site = Site::start(
            self.hosts(),
            source.clone(),
            Settings {
                web_dir,
                served_by: "lab".parse()?,
                access,
            },
        );
        source.feed(site.inputs.clone());

        let router = site.router.merge(self.routes());
        if open {
            return Ok(router);
        }
        let signin = Signin {
            sessions,
            whois: Whois::Fixed(Peer {
                login: viewer.login.clone(),
                device: viewer.device,
                tagged: false,
                capabilities: BTreeSet::new(),
            }),
            policy: Policy {
                capability: String::new(),
                logins: vec![viewer.login],
            },
            origin: origin.to_owned(),
        };
        Ok(router.merge(signin.router()?))
    }

    /// tells the aggregator what every machine says, then runs the clock
    pub fn feed(self: &Arc<Self>, inputs: mpsc::Sender<Input>) {
        if self.inputs.set(inputs).is_err() {
            return;
        }
        let fixture = self.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(Self::TICK);
            loop {
                ticks.tick().await;
                let (at, speed) = {
                    let clock = fixture.clock.lock().unwrap();
                    (clock.at, clock.speed)
                };
                fixture
                    .move_to(at + speed * Self::TICK.as_millis() as f64)
                    .await;
            }
        });
    }

    /// moves the timeline to `t` seconds and runs it at `speed` from there,
    /// answering once the site shows that moment
    pub async fn seek(&self, t: f64, speed: f64) -> Status {
        self.clock.lock().unwrap().speed = speed.max(0.0);
        self.move_to(t.max(0.0) * 1_000.0).await
    }

    async fn move_to(&self, at: f64) -> Status {
        let _moving = self.moving.lock().await;
        let (inputs, status) = {
            let mut clock = self.clock.lock().unwrap();
            clock.move_to(at);
            (clock.tell(), clock.status())
        };
        if let Some(sender) = self.inputs.get() {
            for input in inputs {
                let _ = sender.send(input).await;
            }
            // a seek answers once browsers would see the moment it moved to
            let (settled, published) = tokio::sync::oneshot::channel();
            if sender.send(Input::Settled(settled)).await.is_ok() {
                let _ = published.await;
            }
        }
        self.moved.send_replace(());
        status
    }

    /// job `job`'s log on `host` from offset `from`
    pub fn attach(self: &Arc<Self>, host: &Hostname, job: u64, from: u64) -> Result<Feed> {
        let at = HOSTS
            .iter()
            .position(|name| *name == host.as_str())
            .ok_or_else(|| eyre!("{host} is not in the fixture"))?;
        if self.clock.lock().unwrap().world.job(at, job).is_none() {
            return Err(eyre!("{host} knows no job {job}"));
        }
        Ok(Feed {
            fixture: self.clone(),
            host: at,
            job,
            offset: from as usize,
            started: false,
            ended: false,
            moved: self.moved.subscribe(),
        })
    }

    /// `POST /fixture/seek?t=<seconds>[&speed=<x>]`: moves the timeline and
    /// holds it there, or runs it at `speed`
    pub fn routes(self: &Arc<Self>) -> Router {
        #[derive(Deserialize)]
        struct Seek {
            t: f64,
            #[serde(default)]
            speed: f64,
        }

        async fn seek(
            State(fixture): State<Arc<Fixture>>,
            Query(seek): Query<Seek>,
        ) -> Json<Status> {
            Json(fixture.seek(seek.t, seek.speed).await)
        }

        Router::new()
            .route("/fixture/seek", post(seek))
            .with_state(self.clone())
    }
}

impl Clock {
    fn move_to(&mut self, at: f64) {
        let target = at as i64;
        let turn = target / self.period;
        let local = (target % self.period) / STEP * STEP;
        let base = START + turn * self.period;
        if turn != self.turn || base + local < self.world.now {
            self.world = World::seed(base);
            self.turn = turn;
        }
        while self.world.now < base + local {
            self.world.step();
        }
        self.at = at;
    }

    /// what the machines say that they did not say before
    fn tell(&mut self) -> Vec<Input> {
        let now = self.world.now;
        let mut inputs = Vec::new();
        let hosts: Vec<Hostname> = self.world.hosts().cloned().collect();
        for (at, host) in hosts.into_iter().enumerate() {
            let (reachable, seen) = self.world.reachable(at);
            let view = self.world.view(at);
            let told = self.told[at].as_ref();
            if told.is_some_and(|(was, told)| *was == reachable && *told == view) {
                continue;
            }
            let came = told.is_none_or(|(was, _)| !was);
            let input = |at: i64, event| Input::Fleet {
                at: Timestamp(at),
                event,
            };
            let link = |link| FleetEvent::Link {
                host: host.clone(),
                link,
            };
            let viewed = FleetEvent::View {
                host: host.clone(),
                view: Box::new(view.clone()),
            };
            match reachable {
                true => {
                    if came {
                        inputs.push(input(now, link(Link::Connected)));
                    }
                    inputs.push(input(now, viewed));
                }
                false => {
                    let asleep = Unreach::Offline(Offline {
                        last_seen: Some(seen),
                    });
                    inputs.push(input(seen.0, viewed));
                    inputs.push(input(seen.0, link(Link::Unreachable(asleep))));
                }
            }
            self.told[at] = Some((reachable, view));
        }
        inputs
    }

    fn status(&self) -> Status {
        Status {
            t: self.at / 1_000.0,
            speed: self.speed,
            now: self.world.now,
            period: self.period as f64 / 1_000.0,
        }
    }
}

/// a job's log as the simulation writes it
pub struct Feed {
    fixture: Arc<Fixture>,
    host: usize,
    job: u64,
    offset: usize,
    started: bool,
    ended: bool,
    moved: watch::Receiver<()>,
}

impl Feed {
    /// frames a batch holds at most
    const BATCH: usize = 256;

    pub async fn next(&mut self) -> Result<Option<AttachmentEvent>> {
        loop {
            if let Some(event) = self.read()? {
                return Ok(Some(event));
            }
            if self.ended || self.moved.changed().await.is_err() {
                return Ok(None);
            }
        }
    }

    /// what the log holds past the offset, if anything
    fn read(&mut self) -> Result<Option<AttachmentEvent>> {
        let clock = self.fixture.clock.lock().unwrap();
        let (summary, log) = clock
            .world
            .job(self.host, self.job)
            .ok_or_else(|| eyre!("job {} is gone: the timeline moved back", self.job))?;
        if !self.started {
            self.started = true;
            return Ok(Some(AttachmentEvent::Started(summary.clone())));
        }
        if self.ended {
            return Ok(None);
        }

        let frames = &log.frames;
        if self.offset > frames.len() {
            return Err(eyre!(
                "job {}'s log is shorter: the timeline moved back",
                self.job
            ));
        }
        if self.offset < frames.len() {
            let end = frames.len().min(self.offset + Self::BATCH);
            let batch = frames[self.offset..end].to_vec();
            self.offset = end;
            return Ok(Some(AttachmentEvent::Frames {
                frames: batch,
                end: end as u64,
            }));
        }
        if let JobState::Finished(outcome) = &summary.state {
            self.ended = true;
            return Ok(Some(AttachmentEvent::Ended {
                job: summary.id,
                outcome: outcome.clone(),
            }));
        }
        Ok(None)
    }
}
