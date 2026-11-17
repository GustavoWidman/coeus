use std::{
    fs::{self, Permissions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::Duration,
};

use eyre::Result;
use log::warn;
use serde::Serialize;
use tokio::{
    sync::{mpsc, oneshot, watch},
    time,
};

use crate::{
    common::model::{CommitSha, Pinned, Timestamp},
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
    /// waits this long after startup before the first scheduled look, so a
    /// daemon restarting into a new version does not race the job that
    /// installed it
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

/// someone waiting on a lookup that starts after they asked, for where the
/// followed ref points or why it could not be looked up
pub(super) type Ask = oneshot::Sender<Result<Pinned, String>>;

/// looks up the followed ref on its own task, so a network that is down or
/// slow never holds up the engine, and publishes every commit it resolves,
/// to the engine and to the state file when one is set. it looks when its
/// schedule says so and whenever someone asks, and stops once nobody listens.
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

    /// starts looking, on the schedule of a lookout that started at `started`
    pub fn spawn(
        repo: FlakeRepo,
        settings: FollowSettings,
        started: Timestamp,
    ) -> (watch::Receiver<Option<Pinned>>, mpsc::UnboundedSender<Ask>) {
        let (latest, receiver) = watch::channel(None);
        let (asks, asked) = mpsc::unbounded_channel();
        let lookout = Self {
            schedule: Schedule::new(settings.interval, started),
            repo,
            settings,
        };

        tokio::spawn(lookout.run(latest, asked));
        (receiver, asks)
    }

    async fn run(
        mut self,
        latest: watch::Sender<Option<Pinned>>,
        mut asked: mpsc::UnboundedReceiver<Ask>,
    ) {
        loop {
            let mut waiting = Vec::new();
            let nap = self.schedule.until(Timestamp::now()).min(Self::NAP);
            tokio::select! {
                _ = latest.closed() => return,
                _ = time::sleep(nap) => {}
                Some(ask) = asked.recv() => waiting.push(ask),
            }

            // asks sent while the last lookup ran get this one, which starts
            // after them
            while let Ok(ask) = asked.try_recv() {
                waiting.push(ask);
            }
            if waiting.is_empty() && !self.schedule.due(Timestamp::now()) {
                continue;
            }

            let found = self.look().await;
            if let Ok(pinned) = &found {
                if let Some(path) = &self.settings.state_file {
                    publish(path, pinned, Timestamp::now());
                }
                latest.send_replace(Some(pinned.clone()));
            }
            for ask in waiting {
                let _ = ask.send(found.clone());
            }
        }
    }

    async fn look(&mut self) -> Result<Pinned, String> {
        let git_ref = &self.settings.git_ref;
        let error = match time::timeout(Self::PATIENCE, self.repo.resolve(git_ref)).await {
            Ok(Ok(pinned)) => {
                self.schedule.succeeded(Timestamp::now());
                return Ok(pinned);
            }
            Ok(Err(error)) => format!("{error}"),
            Err(_) => format!("gave up after {}s", Self::PATIENCE.as_secs()),
        };

        let wait = self.schedule.failed(Timestamp::now());
        warn!(
            "cannot follow {git_ref}: {error}; looking again in {}s",
            wait.as_secs()
        );
        Err(error)
    }
}

/// what the state file holds: where the followed ref points and when that
/// was last confirmed
#[derive(Serialize)]
struct Published<'a> {
    #[serde(rename = "ref")]
    git_ref: &'a str,
    rev: CommitSha,
    /// the commit time in unix seconds, exactly the flake's `lastModified`
    committed: i64,
    /// when the lookup succeeded, in unix seconds
    checked: i64,
}

/// writes where `pinned` points to `path`, replacing it whole so a reader
/// never sees half a file. a failure only warns: the file goes stale and
/// readers stop trusting it.
pub(super) fn publish(path: &Path, pinned: &Pinned, checked: Timestamp) {
    let published = Published {
        git_ref: &pinned.git_ref,
        rev: pinned.rev,
        committed: pinned.committed_at.as_secs(),
        checked: checked.as_secs(),
    };
    if let Err(error) = replace(path, &published) {
        warn!("cannot write {}: {error}", path.display());
    }
}

fn replace(path: &Path, published: &Published<'_>) -> Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;

    let mut file = tempfile::Builder::new().tempfile_in(dir)?;
    serde_json::to_writer(&mut file, published)?;
    file.write_all(b"\n")?;
    file.as_file()
        .set_permissions(Permissions::from_mode(0o644))?;
    file.persist(path)?;
    Ok(())
}
