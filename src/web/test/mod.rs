mod card;

use crate::common::model::{
    Action, DaemonState, FdaStatus, HostInfo, Hostname, JobId, JobState, JobSummary, MachineView,
    OrderId, Origin, Platform, Source, SystemInfo, Timestamp,
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
