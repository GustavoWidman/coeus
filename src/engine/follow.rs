use std::ops::Not;

use eyre::{Result, bail};
use tokio::sync::watch;

use super::lookout::Lookout;
use crate::{
    common::model::{Follow, Pinned, Source, Timestamp, valid_ref},
    config::FollowSettings,
    nix::FlakeRepo,
};

/// keeps a machine on a git ref without anyone ordering it: a machine that
/// was off or asleep through a rollout catches up when it comes back.
///
/// it catches up when the ref points at a commit newer than the machine's last
/// deployment, so a hand-ordered deploy of something else holds until the
/// fleet moves on.
pub(super) struct Follower {
    git_ref: String,
    /// what the ref pointed at when last looked up
    latest: watch::Receiver<Option<Pinned>>,
}

/// the machine's last successful deployment
pub(super) struct Deployed<'a> {
    /// what the job that made the running system deployed, or `None` when
    /// the machine was switched outside coeus
    pub source: Option<&'a Source>,
    pub at: Timestamp,
}

impl Follower {
    /// follows `settings.git_ref`, looking it up on a task of its own
    pub fn spawn(repo: FlakeRepo, settings: FollowSettings) -> Result<Self> {
        if !valid_ref(&settings.git_ref) {
            bail!("cannot follow {:?}: not a branch or tag name", settings.git_ref);
        }

        Ok(Self {
            git_ref: settings.git_ref.clone(),
            latest: Lookout::spawn(repo, settings),
        })
    }

    /// a follower that saw `latest` and looks no further, for tests
    #[cfg(test)]
    pub fn seeing(git_ref: &str, latest: Option<Pinned>) -> Self {
        Self {
            git_ref: git_ref.to_owned(),
            latest: watch::channel(latest).1,
        }
    }

    /// the source to deploy to catch up, if the machine is behind
    pub fn behind(&self, deployed: Option<&Deployed<'_>>) -> Option<Source> {
        let latest = self.latest.borrow();
        let latest = latest.as_ref()?;
        holds(latest, deployed)
            .not()
            .then(|| latest.clone().into())
    }

    pub fn state(&self, deployed: Option<&Deployed<'_>>) -> Follow {
        let latest = self.latest.borrow().clone();
        Follow {
            git_ref: self.git_ref.clone(),
            paused: latest.as_ref().is_some_and(|latest| {
                holds(latest, deployed)
                    && deployed.is_some_and(|deployed| !runs(latest, deployed.source))
            }),
            latest,
        }
    }
}

fn holds(latest: &Pinned, deployed: Option<&Deployed<'_>>) -> bool {
    deployed.is_some_and(|deployed| {
        runs(latest, deployed.source) || latest.committed_at <= deployed.at
    })
}

fn runs(latest: &Pinned, source: Option<&Source>) -> bool {
    matches!(source, Some(Source::Git { rev, .. }) if *rev == latest.rev)
}
