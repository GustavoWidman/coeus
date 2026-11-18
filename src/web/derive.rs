//! the dashboard's view, built from what daemons report by pure functions

use super::view::{
    ActionView, DaemonStateView, DiffSummary, DiffView, FollowView, JobCard, JobStateView,
    LinkView, LiveProgress, MachineState, OriginView, OutcomeView, PackageChangeView, PhaseView,
    PinnedView, PlatformView, SourceView, TallyView, UnreachView,
};
use crate::{
    client::{Link, Unreach},
    common::model::{
        Action, ClosureDiff, DaemonState, Follow, JobState, JobSummary, JobView, MachineView,
        Origin, Outcome, Phase, Platform, Source, Tally, Timestamp,
    },
};

/// a machine's link, `since` being when it went out of reach
pub fn link(link: &Link, since: Timestamp) -> LinkView {
    match link {
        Link::Connecting => LinkView::Connecting,
        Link::Connected => LinkView::Connected,
        Link::Unreachable(reason) => LinkView::Unreachable {
            since,
            reason: match reason {
                Unreach::Offline(offline) => UnreachView::Offline {
                    last_seen: offline.last_seen,
                },
                Unreach::Refused(detail) => UnreachView::Refused {
                    detail: detail.clone(),
                },
                Unreach::Failed(detail) => UnreachView::Failed {
                    detail: detail.clone(),
                },
            },
        },
    }
}

/// a machine's view as received at `seen_at`, its running job's progress
/// left to [progress]
pub fn state(view: &MachineView, seen_at: Timestamp) -> MachineState {
    let system = &view.system;
    MachineState {
        seen_at,
        platform: view.host.platform.into(),
        system: view.host.system.clone(),
        version: view.host.version.clone(),
        daemon: view.daemon.into(),
        generation: system.generation,
        current: system.current.as_ref().map(|path| path.as_str().to_owned()),
        pending_boot: system.pending_boot,
        reboot_required: system.reboot_required,
        deployed: system.deployed.as_ref().map(SourceView::from),
        follow: system.follow.as_ref().map(FollowView::from),
        running: view.running.as_ref().map(JobCard::from),
        next: view.next.as_ref().map(JobCard::from),
        recent: view.recent.iter().map(JobCard::from).collect(),
    }
}

/// the running job's numbers, if a job runs
pub fn progress(view: &MachineView) -> Option<LiveProgress> {
    let running = view.running.as_ref()?;
    let progress = &running.progress;
    Some(LiveProgress {
        host: view.host.hostname.clone(),
        job: running.summary.id.0,
        builds: progress.builds.into(),
        downloads: progress.downloads.into(),
        downloaded_bytes: progress.downloaded_bytes,
        expected_bytes: progress.expected_bytes,
        active: progress.active.clone(),
    })
}

impl From<&JobSummary> for JobCard {
    fn from(job: &JobSummary) -> Self {
        Self {
            id: job.id.0,
            order: job.order.to_string(),
            source: (&job.source).into(),
            action: job.action.into(),
            origin: (&job.origin).into(),
            state: (&job.state).into(),
            ordered_at: job.ordered_at,
            started_at: job.started_at,
            finished_at: job.finished_at,
            phase_started_at: None,
            toplevel: job.toplevel.as_ref().map(|path| path.as_str().to_owned()),
            attempt: job.attempt,
            changes: None,
        }
    }
}

impl From<&JobView> for JobCard {
    fn from(view: &JobView) -> Self {
        Self {
            phase_started_at: view.phase_started_at,
            changes: view.changes,
            ..Self::from(&view.summary)
        }
    }
}

impl From<&Source> for SourceView {
    fn from(source: &Source) -> Self {
        match source {
            Source::Git { rev, git_ref } => Self::Git {
                rev: rev.to_string(),
                git_ref: git_ref.clone(),
            },
            Source::Snapshot { nar_hash, label } => Self::Snapshot {
                label: label.clone(),
                nar_hash: nar_hash.as_str().to_owned(),
            },
            Source::Generation(number) => Self::Generation { number: *number },
        }
    }
}

impl From<&Origin> for OriginView {
    fn from(origin: &Origin) -> Self {
        match origin {
            Origin::User { user, host } => Self::User {
                user: user.clone(),
                host: host.clone(),
            },
            Origin::Follow => Self::Follow,
            Origin::GitHub {
                repository,
                workflow,
                run_id,
                attempt,
            } => Self::Github {
                repository: repository.clone(),
                workflow: workflow.clone(),
                run_id: *run_id,
                attempt: *attempt,
                url: origin.url().unwrap_or_default(),
            },
        }
    }
}

impl From<&Follow> for FollowView {
    fn from(follow: &Follow) -> Self {
        Self {
            git_ref: follow.git_ref.clone(),
            latest: follow.latest.as_ref().map(|pinned| PinnedView {
                rev: pinned.rev.to_string(),
                committed_at: pinned.committed_at,
            }),
            paused: follow.paused,
        }
    }
}

impl From<&JobState> for JobStateView {
    fn from(state: &JobState) -> Self {
        match state {
            JobState::Queued => Self::Queued,
            JobState::Running { phase } => Self::Running {
                phase: (*phase).into(),
            },
            JobState::Finished(outcome) => Self::Finished {
                outcome: outcome.into(),
            },
        }
    }
}

impl From<&Outcome> for OutcomeView {
    fn from(outcome: &Outcome) -> Self {
        match outcome {
            Outcome::Succeeded => Self::Succeeded,
            Outcome::UpToDate => Self::UpToDate,
            Outcome::Cancelled => Self::Cancelled,
            Outcome::Interrupted => Self::Interrupted,
            Outcome::Superseded { by } => Self::Superseded { by: by.0 },
            Outcome::Failed(failure) => Self::Failed {
                phase: failure.phase.into(),
                message: failure.message.clone(),
            },
            Outcome::RolledBack(failure) => Self::RolledBack {
                phase: failure.phase.into(),
                message: failure.message.clone(),
            },
        }
    }
}

impl From<Phase> for PhaseView {
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::Preparing => Self::Preparing,
            Phase::Fetching => Self::Fetching,
            Phase::Evaluating => Self::Evaluating,
            Phase::Building => Self::Building,
            Phase::Activating => Self::Activating,
            Phase::Verifying => Self::Verifying,
        }
    }
}

impl From<Action> for ActionView {
    fn from(action: Action) -> Self {
        match action {
            Action::Switch => Self::Switch,
            Action::Boot => Self::Boot,
            Action::Test => Self::Test,
            Action::Build => Self::Build,
        }
    }
}

impl From<Platform> for PlatformView {
    fn from(platform: Platform) -> Self {
        match platform {
            Platform::NixOs => Self::NixOs,
            Platform::Darwin => Self::Darwin,
        }
    }
}

impl From<DaemonState> for DaemonStateView {
    fn from(state: DaemonState) -> Self {
        match state {
            DaemonState::Serving => Self::Serving,
            DaemonState::Restarting => Self::Restarting,
        }
    }
}

impl From<Tally> for TallyView {
    fn from(tally: Tally) -> Self {
        Self {
            done: tally.done,
            expected: tally.expected,
            running: tally.running,
            failed: tally.failed,
        }
    }
}

impl From<&ClosureDiff> for DiffSummary {
    fn from(diff: &ClosureDiff) -> Self {
        Self {
            changes: diff.changes.len() as u32,
            size_delta: diff.size_delta(),
        }
    }
}

impl From<&ClosureDiff> for DiffView {
    fn from(diff: &ClosureDiff) -> Self {
        Self {
            changes: diff
                .changes
                .iter()
                .map(|change| PackageChangeView {
                    name: change.name.clone(),
                    before: change.before.clone(),
                    after: change.after.clone(),
                    size_delta: change.size_delta,
                })
                .collect(),
        }
    }
}
