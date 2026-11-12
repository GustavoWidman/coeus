mod board;
mod plain;
mod summary;
mod text;

use crate::{
    client::FleetEvent,
    common::model::{
        Action, Admission, DaemonState, Failure, FdaStatus, HostInfo, Hostname, JobId, JobState,
        JobSummary, JobView, MachineView, OrderId, Origin, Outcome, Phase, Placement, Platform,
        Progress, Source, SystemInfo, Tally, Timestamp,
    },
    nix::strip_ansi,
    ui::{Board, Heading, board::Ordering},
};

pub const SHA: &str = "caf4e2b9a1d6c3e8f0b2a4c6d8e0f1a3b5c7d9e1";
pub const TOPLEVEL: &str = "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11";

/// ten minutes after the order
pub const NOW: Timestamp = Timestamp(1_800_000_600_000);
pub const ORDERED: Timestamp = Timestamp(1_800_000_000_000);

pub fn host(name: &str) -> Hostname {
    name.parse().unwrap()
}

pub fn source() -> Source {
    Source::Git {
        rev: SHA.parse().unwrap(),
        git_ref: Some("main".into()),
    }
}

pub fn summary(id: u64, state: JobState) -> JobSummary {
    let finished = state.is_finished();
    JobSummary {
        id: JobId(id),
        order: OrderId(7),
        source: source(),
        action: Action::Switch,
        origin: Origin::User {
            user: "r3dlust".into(),
            host: "altar".into(),
        },
        state,
        ordered_at: ORDERED,
        started_at: Some(Timestamp(ORDERED.0 + 1_000)),
        finished_at: finished.then_some(Timestamp(ORDERED.0 + 241_000)),
        toplevel: None,
        attempt: 1,
    }
}

pub fn failed(id: u64) -> JobSummary {
    let mut failure = Failure::new(
        Phase::Building,
        "builder for 'hello.drv' failed with exit code 1",
    );
    failure.tail = vec!["make: *** [all] Error 1".into()];
    summary(id, JobState::Finished(Outcome::Failed(failure)))
}

pub fn building(id: u64) -> JobView {
    JobView {
        summary: summary(
            id,
            JobState::Running {
                phase: Phase::Building,
            },
        ),
        progress: Progress {
            builds: Tally {
                done: 142,
                expected: 225,
                running: 2,
                failed: 0,
            },
            downloads: Tally {
                done: 12,
                expected: 40,
                running: 1,
                failed: 0,
            },
            downloaded_bytes: 1_288_490_189,
            expected_bytes: 3_650_722_201,
            active: vec!["firefox-121.0".into(), "linux-6.12".into()],
        },
        phase_started_at: Some(Timestamp(ORDERED.0 + 60_000)),
        changes: None,
    }
}

pub fn view(name: &str, running: Option<JobView>, recent: Vec<JobSummary>) -> MachineView {
    MachineView {
        host: HostInfo {
            hostname: host(name),
            platform: Platform::NixOs,
            system: "x86_64-linux".into(),
            version: "0.1.0".into(),
        },
        system: SystemInfo {
            current: Some(TOPLEVEL.parse().unwrap()),
            generation: Some(41),
            pending_boot: false,
            reboot_required: false,
            deployed: Some(source()),
            follow: None,
            fda: FdaStatus::NotApplicable,
            checked_at: NOW,
        },
        daemon: DaemonState::Serving,
        running,
        next: None,
        recent,
    }
}

pub fn order_board(hosts: &[&str]) -> Board {
    let hosts: Vec<Hostname> = hosts.iter().map(|name| host(name)).collect();
    Board::new(
        Heading::Order {
            id: OrderId(0x1f2e_3d4c_5b6a_7980),
            source: source(),
            action: Action::Switch,
            since: ORDERED,
        },
        &hosts,
    )
}

pub fn watch_board(hosts: &[&str]) -> Board {
    let hosts: Vec<Hostname> = hosts.iter().map(|name| host(name)).collect();
    Board::new(Heading::Watch { since: ORDERED }, &hosts)
}

pub fn admit(board: &mut Board, name: &str, job: u64) {
    board.order(
        &host(name),
        Ordering::Admitted(Admission {
            job: JobId(job),
            placement: Placement::Started,
        }),
    );
}

pub fn show(board: &mut Board, view: MachineView) {
    board.apply(FleetEvent::View {
        host: view.host.hostname.clone(),
        view: Box::new(view),
    });
}

pub fn plain(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| strip_ansi(line).into_owned())
        .collect()
}
