//! the dashboard's view, built from what daemons report by pure functions

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use super::view::{
    ActionView, DaemonStateView, DeploymentRow, DeploymentView, DiffSummary, DiffView, FollowView,
    JobCard, JobStateView, LinkView, LiveProgress, MachineCard, MachineState, OriginView,
    OutcomeView, PackageChangeView, PhaseView, PinnedView, PlatformView, SourceView, TallyView,
    UnreachView, Verdict, WaveView,
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

/// how far back the dashboard shows deployments, in milliseconds
const DEPLOYMENT_AGE: i64 = 24 * 60 * 60 * 1000;
const DEPLOYMENTS: usize = 5;
/// `coeus deploy` orders a wave's machines all at once, so a longer pause
/// between two of an order's jobs starts the next wave
const WAVE_GAP: i64 = 2_000;

/// the fleet's newest deployments, newest first. daemons know only jobs and
/// the order each belongs to, so a deployment is the jobs that share an order,
/// split into waves where the orders pause, plus the machines that take its
/// commit by following a ref.
pub fn deployments(cards: &[MachineCard], now: Timestamp) -> Vec<DeploymentView> {
    let mut orders: BTreeMap<&str, Vec<Ordered>> = BTreeMap::new();
    for (position, card) in cards.iter().enumerate() {
        let Some(state) = &card.state else {
            continue;
        };
        for job in jobs(state).filter(|job| job.origin != OriginView::Follow) {
            orders.entry(&job.order).or_default().push(Ordered {
                card,
                position,
                job,
            });
        }
    }

    let mut orders: Vec<Vec<Ordered>> = orders
        .into_values()
        .map(|mut jobs| {
            jobs.sort_by_key(|ordered| (ordered.job.ordered_at, ordered.position));
            jobs
        })
        .filter(|jobs| now.0.saturating_sub(jobs[0].job.ordered_at.0) < DEPLOYMENT_AGE)
        .collect();
    orders.sort_by_key(|jobs| Reverse(jobs[0].job.ordered_at));
    orders.truncate(DEPLOYMENTS);

    orders.iter().map(|jobs| deployment(jobs, cards)).collect()
}

/// a machine's job of an order, `position` being the machine's place in the
/// inventory
struct Ordered<'a> {
    card: &'a MachineCard,
    position: usize,
    job: &'a JobCard,
}

/// `jobs` in the order they were ordered
fn deployment(jobs: &[Ordered], cards: &[MachineCard]) -> DeploymentView {
    let first = jobs[0].job;
    let waves: Vec<&[Ordered]> = jobs
        .chunk_by(|earlier, later| later.job.ordered_at.0 - earlier.job.ordered_at.0 <= WAVE_GAP)
        .collect();

    let mut rows: Vec<DeploymentRow> = waves
        .iter()
        .enumerate()
        .flat_map(|(wave, jobs)| {
            jobs.iter().map(move |ordered| DeploymentRow {
                host: ordered.card.host.clone(),
                wave: Some(wave as u32),
                verdict: verdict(&ordered.job.state),
                job: Some(ordered.job.id),
                diff: None,
            })
        })
        .collect();
    if let SourceView::Git { rev, git_ref } = &first.source {
        let ordered = |card: &MachineCard| jobs.iter().any(|job| job.card.host == card.host);
        let others: Vec<&MachineCard> = cards.iter().filter(|card| !ordered(card)).collect();
        rows.extend(followers(cards, &others, rev, git_ref.as_deref()));
    }

    DeploymentView {
        order: first.order.clone(),
        source: first.source.clone(),
        origin: first.origin.clone(),
        action: first.action,
        subject: None,
        ordered_at: first.ordered_at,
        settled_at: settled(jobs),
        waves: waves
            .iter()
            .map(|jobs| WaveView {
                name: None,
                hosts: jobs
                    .iter()
                    .map(|ordered| ordered.card.host.clone())
                    .collect(),
                started_at: jobs[0].job.ordered_at,
                settled_at: settled(jobs),
            })
            .collect(),
        rows,
    }
}

fn verdict(state: &JobStateView) -> Verdict {
    match state {
        JobStateView::Queued => Verdict::Waiting,
        JobStateView::Running { .. } => Verdict::Running,
        JobStateView::Finished {
            outcome: OutcomeView::Succeeded | OutcomeView::UpToDate | OutcomeView::Superseded { .. },
        } => Verdict::Done,
        JobStateView::Finished { .. } => Verdict::Failed,
    }
}

/// when the last of `jobs` finished, once all have
fn settled(jobs: &[Ordered]) -> Option<Timestamp> {
    jobs.iter()
        .map(|ordered| ordered.job.finished_at)
        .try_fold(Timestamp(i64::MIN), |last, finished| {
            Some(last.max(finished?))
        })
}

/// rows for the machines among `others` that take commit `rev` by following
/// a ref: the ref it was deployed from, or any ref a machine of the fleet
/// found pointing at it. a deploy from CI orders `main`, then moves the
/// `deployed` ref that sleeping machines follow to the same commit.
fn followers(
    cards: &[MachineCard],
    others: &[&MachineCard],
    rev: &str,
    git_ref: Option<&str>,
) -> Vec<DeploymentRow> {
    let states = || cards.iter().filter_map(|card| card.state.as_ref());
    let pointing = states().filter_map(|state| {
        let follow = state.follow.as_ref()?;
        (follow.latest.as_ref()?.rev == rev).then_some(follow.git_ref.as_str())
    });
    let caught_up = states()
        .flat_map(jobs)
        .filter_map(|job| catch_up_ref(job, rev));
    let refs: BTreeSet<&str> = git_ref
        .into_iter()
        .chain(pointing)
        .chain(caught_up)
        .collect();

    others
        .iter()
        .filter_map(|card| {
            let state = card.state.as_ref()?;
            let follow = state.follow.as_ref()?;
            (!follow.paused && refs.contains(follow.git_ref.as_str()))
                .then(|| follower(card, state, rev))
        })
        .collect()
}

/// the ref a catch-up job to `rev` followed
fn catch_up_ref<'a>(job: &'a JobCard, rev: &str) -> Option<&'a str> {
    match (&job.origin, &job.source) {
        (
            OriginView::Follow,
            SourceView::Git {
                rev: caught,
                git_ref: Some(git_ref),
            },
        ) if caught == rev => Some(git_ref),
        _ => None,
    }
}

fn follower(card: &MachineCard, state: &MachineState, rev: &str) -> DeploymentRow {
    let catch_up = jobs(state)
        .filter(|job| catch_up_ref(job, rev).is_some())
        .max_by_key(|job| job.id);
    let runs_it =
        matches!(&state.deployed, Some(SourceView::Git { rev: deployed, .. }) if deployed == rev);

    let verdict = match catch_up.map(|job| &job.state) {
        Some(JobStateView::Queued | JobStateView::Running { .. }) => Verdict::CatchingUp,
        Some(JobStateView::Finished {
            outcome: OutcomeView::Succeeded | OutcomeView::UpToDate,
        }) => Verdict::CaughtUp,
        Some(JobStateView::Finished {
            outcome: OutcomeView::Superseded { .. },
        })
        | None => match runs_it {
            true => Verdict::CaughtUp,
            false => Verdict::Pending,
        },
        Some(JobStateView::Finished { .. }) => Verdict::Failed,
    };

    DeploymentRow {
        host: card.host.clone(),
        wave: None,
        verdict,
        job: catch_up.map(|job| job.id),
        diff: None,
    }
}

/// every job a machine reported, wherever it sits
fn jobs(state: &MachineState) -> impl Iterator<Item = &JobCard> {
    state.running.iter().chain(&state.next).chain(&state.recent)
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
