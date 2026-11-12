use std::time::Duration;

use log::warn;
use tokio::{sync::watch, time};

use crate::{
    common::model::{Pinned, Timestamp},
    config::FollowSettings,
    nix::FlakeRepo,
};

/// when to look up the followed ref next. it keeps wall-clock time, which
/// jumps ahead across sleep, so a machine that wakes up looks right away, and
/// it looks again sooner after a failure, since a machine that just booted or
/// woke up often has no network yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Schedule {
    interval: Duration,
    next: Timestamp,
    failures: u32,
}

impl Schedule {
    /// waits this long after startup before the first look, so a daemon
    /// restarting into a new version does not race the job that installed it
    pub const SETTLE: Duration = Duration::from_secs(30);
    /// the wait after the first failure, doubling after each one up to the
    /// interval
    pub const RETRY: Duration = Duration::from_secs(30);

    pub fn new(interval: Duration, now: Timestamp) -> Self {
        Self {
            interval,
            next: now.after(Self::SETTLE),
            failures: 0,
        }
    }

    pub fn due(&self, now: Timestamp) -> bool {
        now >= self.next
    }

    /// how long until the next look
    pub fn until(&self, now: Timestamp) -> Duration {
        now.until(self.next)
    }

    pub fn succeeded(&mut self, now: Timestamp) {
        self.failures = 0;
        self.next = now.after(self.interval);
    }

    /// schedules the next look after a failure, returning how far off it is
    pub fn failed(&mut self, now: Timestamp) -> Duration {
        let wait = Self::RETRY
            .saturating_mul(1 << self.failures.min(16))
            .min(self.interval);
        self.failures += 1;
        self.next = now.after(wait);
        wait
    }
}

/// looks up the followed ref on its own task, so a network that is down or
/// slow never holds up the engine, and publishes every commit it resolves.
/// it stops once nobody listens.
pub(super) struct Lookout {
    repo: FlakeRepo,
    settings: FollowSettings,
    schedule: Schedule,
}

impl Lookout {
    /// gives up on a lookup that takes longer, like one stuck connecting
    const PATIENCE: Duration = Duration::from_secs(120);
    /// the longest nap between glances at the wall clock
    const NAP: Duration = Duration::from_secs(5);

    pub fn spawn(repo: FlakeRepo, settings: FollowSettings) -> watch::Receiver<Option<Pinned>> {
        let (latest, receiver) = watch::channel(None);
        let lookout = Self {
            schedule: Schedule::new(settings.interval, Timestamp::now()),
            repo,
            settings,
        };

        tokio::spawn(lookout.run(latest));
        receiver
    }

    async fn run(mut self, latest: watch::Sender<Option<Pinned>>) {
        loop {
            let nap = self.schedule.until(Timestamp::now()).min(Self::NAP);
            tokio::select! {
                _ = latest.closed() => return,
                _ = time::sleep(nap) => {}
            }

            if !self.schedule.due(Timestamp::now()) {
                continue;
            }

            let git_ref = &self.settings.git_ref;
            let error = match time::timeout(Self::PATIENCE, self.repo.resolve(git_ref)).await {
                Ok(Ok(pinned)) => {
                    self.schedule.succeeded(Timestamp::now());
                    latest.send_replace(Some(pinned));
                    continue;
                }
                Ok(Err(error)) => format!("{error}"),
                Err(_) => format!("gave up after {}s", Self::PATIENCE.as_secs()),
            };

            let wait = self.schedule.failed(Timestamp::now());
            warn!(
                "cannot follow {git_ref}: {error}; looking again in {}s",
                wait.as_secs()
            );
        }
    }
}
