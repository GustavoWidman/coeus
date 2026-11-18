mod card;
mod connector;
mod contract;
mod deployment;
mod fixture;
mod live;
mod ready;
mod session;
mod site;
mod sse;
mod stream;
mod transcript;
mod typescript;

use crate::{
    common::model::{
        Action, DaemonState, FdaStatus, Follow, HostInfo, Hostname, JobId, JobState, JobSummary,
        JobView, MachineView, OrderId, Origin, Outcome, Phase, Pinned, Platform, Source,
        SystemInfo, Timestamp,
    },
    web::{
        derive,
        view::{LinkView, MachineCard},
    },
};

pub const SHA: &str = "caf4e2b9a1d6c3e8f0b2a4c6d8e0f1a3b5c7d9e1";
pub const ORDERED: Timestamp = Timestamp(1_800_000_000_000);

pub fn host(name: &str) -> Hostname {
    name.parse().unwrap()
}

pub fn git(rev: &str, git_ref: &str) -> Source {
    Source::Git {
        rev: rev.parse().unwrap(),
        git_ref: Some(git_ref.into()),
    }
}

pub fn user() -> Origin {
    Origin::User {
        user: "r3dlust".into(),
        host: "altar".into(),
    }
}

/// job `id` of `order`, ordered `at` milliseconds after [ORDERED]
pub fn job(id: u64, order: u64, at: i64, state: JobState) -> JobSummary {
    let finished = state.is_finished();
    let ordered_at = Timestamp(ORDERED.0 + at);
    JobSummary {
        id: JobId(id),
        order: OrderId(order),
        source: git(SHA, "main"),
        action: Action::Switch,
        origin: user(),
        state,
        ordered_at,
        started_at: Some(Timestamp(ordered_at.0 + 1_000)),
        finished_at: finished.then_some(Timestamp(ordered_at.0 + 60_000)),
        toplevel: None,
        attempt: 1,
    }
}

pub fn view(name: &str) -> MachineView {
    MachineView {
        host: HostInfo {
            hostname: host(name),
            platform: Platform::NixOs,
            system: "x86_64-linux".into(),
            version: "0.5.1".into(),
        },
        system: SystemInfo {
            current: None,
            generation: Some(41),
            pending_boot: false,
            reboot_required: false,
            deployed: None,
            follow: None,
            fda: FdaStatus::NotApplicable,
            checked_at: ORDERED,
        },
        daemon: DaemonState::Serving,
        running: None,
        next: None,
        recent: Vec::new(),
    }
}

/// the commit a deploy rolls out; [SHA] is what machines ran before
pub const RELEASE: &str = "1111111111111111111111111111111111111111";
pub const NOW: Timestamp = Timestamp(ORDERED.0 + 600_000);

pub fn succeeded() -> JobState {
    JobState::Finished(Outcome::Succeeded)
}

pub fn running() -> JobState {
    JobState::Running {
        phase: Phase::Building,
    }
}

/// job `id` of order `order` deploying [RELEASE] from `main`
pub fn release(id: u64, order: u64, at: i64, state: JobState) -> JobSummary {
    JobSummary {
        source: git(RELEASE, "main"),
        ..job(id, order, at, state)
    }
}

/// a catch-up to [RELEASE] through `git_ref`
pub fn catch_up(id: u64, git_ref: &str, state: JobState) -> JobSummary {
    JobSummary {
        source: git(RELEASE, git_ref),
        origin: Origin::Follow,
        ..job(id, 0xf011, 120_000, state)
    }
}

/// a machine reporting `jobs`, each where its state puts it
pub fn machine(name: &str, jobs: Vec<JobSummary>) -> MachineView {
    let mut machine = view(name);
    for job in jobs {
        match job.state {
            JobState::Queued => machine.next = Some(job),
            JobState::Running { .. } => machine.running = Some(JobView::queued(job)),
            JobState::Finished(_) => machine.recent.push(job),
        }
    }
    machine
}

pub fn following(mut machine: MachineView, git_ref: &str, latest: &str) -> MachineView {
    machine.system.follow = Some(Follow {
        git_ref: git_ref.into(),
        latest: Some(Pinned {
            git_ref: git_ref.into(),
            rev: latest.parse().unwrap(),
            committed_at: ORDERED,
        }),
        paused: false,
    });
    machine
}

pub fn card(machine: &MachineView) -> MachineCard {
    MachineCard {
        host: machine.host.hostname.clone(),
        link: LinkView::Connected,
        state: Some(derive::state(machine, ORDERED)),
    }
}
