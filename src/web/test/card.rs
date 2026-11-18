use super::{ORDERED, SHA, git, host, job, view};
use crate::{
    client::{Link, Offline, Unreach},
    common::model::{
        ClosureDiff, Failure, Follow, JobId, JobState, JobView, Origin, Outcome, PackageChange,
        Phase, Pinned, Platform, Progress, Tally, Timestamp,
    },
    web::{
        derive,
        view::{
            ActionView, DaemonStateView, DiffSummary, FollowView, JobCard, JobStateView, LinkView,
            LiveProgress, MachineState, OriginView, OutcomeView, PhaseView, PinnedView,
            PlatformView, SourceView, TallyView, UnreachView,
        },
    },
};

const TOPLEVEL: &str = "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11";
const SEEN: Timestamp = Timestamp(ORDERED.0 + 300_000);

fn tally(done: u32, expected: u32) -> Tally {
    Tally {
        done,
        expected,
        running: 1,
        failed: 0,
    }
}

fn building() -> JobView {
    let mut summary = job(
        62,
        0x00ab_cdef_0123_4567,
        0,
        JobState::Running {
            phase: Phase::Building,
        },
    );
    summary.toplevel = Some(TOPLEVEL.parse().unwrap());
    JobView {
        summary,
        progress: Progress {
            builds: tally(186, 388),
            downloads: tally(12, 40),
            downloaded_bytes: 1_288_490_189,
            expected_bytes: 3_650_722_201,
            active: vec!["firefox-121.0".into()],
        },
        phase_started_at: Some(Timestamp(ORDERED.0 + 90_000)),
        changes: Some(14),
    }
}

fn card(id: u64, order: &str, state: JobStateView, finished: bool) -> JobCard {
    JobCard {
        id,
        order: order.into(),
        source: SourceView::Git {
            rev: SHA.into(),
            git_ref: Some("main".into()),
        },
        action: ActionView::Switch,
        origin: OriginView::User {
            user: "r3dlust".into(),
            host: "altar".into(),
        },
        state,
        ordered_at: ORDERED,
        started_at: Some(Timestamp(ORDERED.0 + 1_000)),
        finished_at: finished.then_some(Timestamp(ORDERED.0 + 60_000)),
        phase_started_at: None,
        toplevel: None,
        attempt: 1,
        changes: None,
    }
}

#[test]
fn a_machine_keeps_its_jobs_where_its_daemon_put_them() {
    let mut machine = view("lab");
    machine.host.platform = Platform::Darwin;
    machine.system.current = Some(TOPLEVEL.parse().unwrap());
    machine.system.deployed = Some(git(SHA, "main"));
    machine.system.follow = Some(Follow {
        git_ref: "deployed".into(),
        latest: Some(Pinned {
            git_ref: "deployed".into(),
            rev: SHA.parse().unwrap(),
            committed_at: Timestamp(ORDERED.0 - 5_000),
        }),
        paused: true,
    });
    machine.running = Some(building());
    machine.next = Some(job(63, 9, 0, JobState::Queued));
    let mut failure = Failure::new(Phase::Building, "builder for 'zfs.drv' failed");
    failure.tail = vec!["make: *** [all] Error 1".into()];
    let mut superseded = job(
        60,
        8,
        0,
        JobState::Finished(Outcome::Superseded { by: JobId(61) }),
    );
    superseded.origin = Origin::GitHub {
        repository: "r3dlust/nix".into(),
        workflow: "deploy".into(),
        run_id: 7,
        attempt: 2,
    };
    machine.recent = vec![
        job(61, 8, 0, JobState::Finished(Outcome::Failed(failure))),
        superseded,
    ];

    let mut running = card(
        62,
        "00abcdef01234567",
        JobStateView::Running {
            phase: PhaseView::Building,
        },
        false,
    );
    running.toplevel = Some(TOPLEVEL.into());
    running.phase_started_at = Some(Timestamp(ORDERED.0 + 90_000));
    running.changes = Some(14);
    let mut github = card(
        60,
        "0000000000000008",
        JobStateView::Finished {
            outcome: OutcomeView::Superseded { by: 61 },
        },
        true,
    );
    github.origin = OriginView::Github {
        repository: "r3dlust/nix".into(),
        workflow: "deploy".into(),
        run_id: 7,
        attempt: 2,
        url: "https://github.com/r3dlust/nix/actions/runs/7".into(),
    };

    assert_eq!(
        derive::state(&machine, SEEN),
        MachineState {
            seen_at: SEEN,
            platform: PlatformView::Darwin,
            system: "x86_64-linux".into(),
            version: "0.5.1".into(),
            daemon: DaemonStateView::Serving,
            generation: Some(41),
            current: Some(TOPLEVEL.into()),
            pending_boot: false,
            reboot_required: false,
            deployed: Some(SourceView::Git {
                rev: SHA.into(),
                git_ref: Some("main".into()),
            }),
            follow: Some(FollowView {
                git_ref: "deployed".into(),
                latest: Some(PinnedView {
                    rev: SHA.into(),
                    committed_at: Timestamp(ORDERED.0 - 5_000),
                }),
                paused: true,
            }),
            running: Some(running),
            next: Some(card(63, "0000000000000009", JobStateView::Queued, false)),
            recent: vec![
                card(
                    61,
                    "0000000000000008",
                    JobStateView::Finished {
                        outcome: OutcomeView::Failed {
                            phase: PhaseView::Building,
                            message: "builder for 'zfs.drv' failed".into(),
                        },
                    },
                    true,
                ),
                github,
            ],
        }
    );
}

#[test]
fn an_order_id_too_large_for_javascript_travels_as_hex() {
    let mut machine = view("lab");
    machine.next = Some(job(1, u64::MAX - 1, 0, JobState::Queued));

    let state = derive::state(&machine, SEEN);

    assert_eq!(state.next.unwrap().order, "fffffffffffffffe");
}

#[test]
fn links_say_why_a_machine_is_out_of_reach() {
    let since = Timestamp(ORDERED.0 + 1_000);
    let cases = [
        (Link::Connecting, LinkView::Connecting),
        (Link::Connected, LinkView::Connected),
        (
            Link::Unreachable(Unreach::Offline(Offline {
                last_seen: Some(ORDERED),
            })),
            LinkView::Unreachable {
                since,
                reason: UnreachView::Offline {
                    last_seen: Some(ORDERED),
                },
            },
        ),
        (
            Link::Unreachable(Unreach::Offline(Offline { last_seen: None })),
            LinkView::Unreachable {
                since,
                reason: UnreachView::Offline { last_seen: None },
            },
        ),
        (
            Link::Unreachable(Unreach::Refused("handshake failed".into())),
            LinkView::Unreachable {
                since,
                reason: UnreachView::Refused {
                    detail: "handshake failed".into(),
                },
            },
        ),
        (
            Link::Unreachable(Unreach::Failed("timed out".into())),
            LinkView::Unreachable {
                since,
                reason: UnreachView::Failed {
                    detail: "timed out".into(),
                },
            },
        ),
    ];

    for (link, expected) in cases {
        assert_eq!(derive::link(&link, since), expected, "{link:?}");
    }
}

#[test]
fn only_a_running_job_has_progress() {
    let mut machine = view("lab");
    assert_eq!(derive::progress(&machine), None);

    machine.running = Some(building());
    let tally = |done, expected| TallyView {
        done,
        expected,
        running: 1,
        failed: 0,
    };
    assert_eq!(
        derive::progress(&machine),
        Some(LiveProgress {
            host: host("lab"),
            job: 62,
            builds: tally(186, 388),
            downloads: tally(12, 40),
            downloaded_bytes: 1_288_490_189,
            expected_bytes: 3_650_722_201,
            active: vec!["firefox-121.0".into()],
        })
    );
}

#[test]
fn a_diff_summary_counts_packages_and_net_growth() {
    let change = |name: &str, size_delta| PackageChange {
        name: name.into(),
        before: vec!["1.0".into()],
        after: vec!["1.1".into()],
        size_delta,
    };
    let diff = ClosureDiff {
        changes: vec![change("firefox", 1_200_000), change("zfs", -200_000)],
    };

    assert_eq!(
        DiffSummary::from(&diff),
        DiffSummary {
            changes: 2,
            size_delta: 1_000_000,
        }
    );
}
