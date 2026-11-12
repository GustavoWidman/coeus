use std::{
    ops::Not,
    time::{Duration, Instant},
};

use log::warn;

use crate::{
    common::model::{Follow, Pinned, Source, Timestamp},
    config::FollowSettings,
    nix::FlakeRepo,
};

/// keeps a machine on a git ref without anyone ordering it: a machine that
/// slept through a rollout catches up when it wakes.
///
/// it catches up when the ref points at a commit newer than the machine's last
/// deployment, so a hand-ordered deploy of something else holds until the
/// fleet moves on.
pub(super) struct Follower {
    repo: FlakeRepo,
    settings: FollowSettings,
    checked: Option<Instant>,
    latest: Option<Pinned>,
}

/// the machine's last successful deployment
pub(super) struct Deployed<'a> {
    pub source: &'a Source,
    pub at: Timestamp,
}

impl Follower {
    /// waits this long after startup before the first check, so a daemon
    /// restarting into a new version does not race the job that installed it
    const SETTLE: Duration = Duration::from_secs(30);

    pub fn new(repo: FlakeRepo, settings: FollowSettings) -> Self {
        Self {
            repo,
            settings,
            checked: Some(Instant::now() - Self::SETTLE),
            latest: None,
        }
    }

    pub fn due(&self) -> bool {
        self.checked
            .is_none_or(|checked| checked.elapsed() >= self.settings.interval)
    }

    pub async fn check(&mut self) {
        self.checked = Some(Instant::now());
        match self.repo.resolve(&self.settings.git_ref).await {
            Ok(pinned) => self.latest = Some(pinned),
            Err(error) => warn!("cannot follow {}: {error}", self.settings.git_ref),
        }
    }

    /// the source to deploy to catch up, if the machine is behind
    pub fn behind(&self, deployed: Option<&Deployed<'_>>) -> Option<Source> {
        let latest = self.latest.as_ref()?;
        self.holds(latest, deployed)
            .not()
            .then(|| latest.clone().into())
    }

    pub fn state(&self, deployed: Option<&Deployed<'_>>) -> Follow {
        Follow {
            git_ref: self.settings.git_ref.clone(),
            latest: self.latest.clone(),
            paused: self.latest.as_ref().is_some_and(|latest| {
                self.holds(latest, deployed)
                    && deployed.is_some_and(|deployed| !runs(latest, deployed.source))
            }),
        }
    }

    fn holds(&self, latest: &Pinned, deployed: Option<&Deployed<'_>>) -> bool {
        deployed.is_some_and(|deployed| {
            runs(latest, deployed.source) || latest.committed_at <= deployed.at
        })
    }
}

fn runs(latest: &Pinned, source: &Source) -> bool {
    matches!(source, Source::Git { rev, .. } if *rev == latest.rev)
}
