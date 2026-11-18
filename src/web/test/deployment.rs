use super::{
    NOW, ORDERED, RELEASE, SHA, card, catch_up, following, git, host, job, machine, release,
    running, succeeded,
};
use crate::{
    common::model::{Failure, JobState, JobSummary, Outcome, Phase, Source, Timestamp},
    web::{
        derive,
        view::{
            ActionView, DeploymentRow, DeploymentView, LinkView, MachineCard, OriginView,
            SourceView, UnreachView, Verdict, WaveView,
        },
    },
};

fn row(name: &str, wave: Option<u32>, verdict: Verdict, job: Option<u64>) -> DeploymentRow {
    DeploymentRow {
        host: host(name),
        wave,
        verdict,
        job,
        diff: None,
    }
}

fn finished_at(at: i64) -> Option<Timestamp> {
    Some(Timestamp(ORDERED.0 + at + 60_000))
}

#[test]
fn an_order_splits_into_waves_where_its_jobs_pause() {
    let failed = JobState::Finished(Outcome::Failed(Failure::new(Phase::Building, "boom")));
    let cards = [
        card(&machine("lab", vec![release(10, 0xa1, 0, succeeded())])),
        card(&machine("nova", vec![release(20, 0xa1, 800, failed)])),
        card(&machine("altar", vec![release(30, 0xa1, 6_000, running())])),
        card(&machine(
            "oracle",
            vec![release(40, 0xa1, 6_500, JobState::Queued)],
        )),
    ];

    assert_eq!(
        derive::deployments(&cards, NOW),
        vec![DeploymentView {
            order: "00000000000000a1".into(),
            source: SourceView::Git {
                rev: RELEASE.into(),
                git_ref: Some("main".into()),
            },
            origin: OriginView::User {
                user: "r3dlust".into(),
                host: "altar".into(),
            },
            action: ActionView::Switch,
            subject: None,
            ordered_at: ORDERED,
            settled_at: None,
            waves: vec![
                WaveView {
                    name: None,
                    hosts: vec![host("lab"), host("nova")],
                    started_at: ORDERED,
                    settled_at: finished_at(800),
                },
                WaveView {
                    name: None,
                    hosts: vec![host("altar"), host("oracle")],
                    started_at: Timestamp(ORDERED.0 + 6_000),
                    settled_at: None,
                },
            ],
            rows: vec![
                row("lab", Some(0), Verdict::Done, Some(10)),
                row("nova", Some(0), Verdict::Failed, Some(20)),
                row("altar", Some(1), Verdict::Running, Some(30)),
                row("oracle", Some(1), Verdict::Waiting, Some(40)),
            ],
        }]
    );
}

#[test]
fn a_deployment_settles_when_its_last_job_finishes() {
    let cards = [
        card(&machine("lab", vec![release(10, 0xa1, 0, succeeded())])),
        card(&machine(
            "nova",
            vec![release(20, 0xa1, 3_000, succeeded())],
        )),
    ];

    let deployment = &derive::deployments(&cards, NOW)[0];

    assert_eq!(deployment.settled_at, finished_at(3_000));
    assert_eq!(deployment.waves.len(), 2);
}

#[test]
fn only_the_five_newest_orders_of_the_last_day_show() {
    let day = 24 * 60 * 60 * 1000;
    let mut jobs: Vec<JobSummary> = (1..=7)
        .map(|order| release(order, order, order as i64 * 60_000, succeeded()))
        .collect();
    jobs.push(release(8, 8, NOW.0 - ORDERED.0 - day - 1, succeeded()));
    jobs.push(catch_up(9, "deployed", succeeded()));
    let cards = [card(&machine("lab", jobs))];

    let orders: Vec<String> = derive::deployments(&cards, NOW)
        .into_iter()
        .map(|deployment| deployment.order)
        .collect();

    assert_eq!(
        orders,
        [7, 6, 5, 4, 3].map(|order: u64| format!("{order:016x}"))
    );
}

#[test]
fn machines_following_the_rolled_out_ref_join_the_deployment() {
    let mut lab = machine("lab", vec![release(10, 0xa1, 0, succeeded())]);
    // lab looked `deployed` up after the deploy moved it
    lab = following(lab, "deployed", RELEASE);
    let mut asleep = card(&following(machine("asleep", vec![]), "deployed", SHA));
    asleep.link = LinkView::Unreachable {
        since: ORDERED,
        reason: UnreachView::Offline {
            last_seen: Some(ORDERED),
        },
    };
    let mut current = following(machine("current", vec![]), "deployed", RELEASE);
    current.system.deployed = Some(git(RELEASE, "deployed"));
    let mut held = following(machine("held", vec![]), "deployed", SHA);
    held.system.follow.as_mut().unwrap().paused = true;
    let failed = JobState::Finished(Outcome::Failed(Failure::new(Phase::Fetching, "offline")));
    let never = MachineCard {
        host: host("never"),
        link: LinkView::Connecting,
        state: None,
    };

    let cards = [
        card(&lab),
        asleep,
        card(&following(
            machine("waking", vec![catch_up(50, "deployed", running())]),
            "deployed",
            RELEASE,
        )),
        card(&following(
            machine("caught", vec![catch_up(60, "deployed", succeeded())]),
            "deployed",
            RELEASE,
        )),
        card(&following(
            machine("broken", vec![catch_up(70, "deployed", failed)]),
            "deployed",
            RELEASE,
        )),
        card(&current),
        card(&following(machine("mainline", vec![]), "main", SHA)),
        card(&held),
        card(&following(machine("staging", vec![]), "staging", SHA)),
        card(&machine("plain", vec![])),
        never,
    ];

    assert_eq!(
        derive::deployments(&cards, NOW)[0].rows,
        vec![
            row("lab", Some(0), Verdict::Done, Some(10)),
            row("asleep", None, Verdict::Pending, None),
            row("waking", None, Verdict::CatchingUp, Some(50)),
            row("caught", None, Verdict::CaughtUp, Some(60)),
            row("broken", None, Verdict::Failed, Some(70)),
            row("current", None, Verdict::CaughtUp, None),
            row("mainline", None, Verdict::Pending, None),
        ]
    );
}

#[test]
fn a_catch_up_job_tells_which_ref_points_at_the_commit() {
    let cards = [
        card(&machine("lab", vec![release(10, 0xa1, 0, succeeded())])),
        card(&following(
            machine("waking", vec![catch_up(50, "deployed", running())]),
            "deployed",
            SHA,
        )),
        card(&following(machine("asleep", vec![]), "deployed", SHA)),
    ];

    assert_eq!(
        derive::deployments(&cards, NOW)[0].rows,
        vec![
            row("lab", Some(0), Verdict::Done, Some(10)),
            row("waking", None, Verdict::CatchingUp, Some(50)),
            row("asleep", None, Verdict::Pending, None),
        ]
    );
}

#[test]
fn a_rollback_has_no_followers() {
    let rollback = JobSummary {
        source: Source::Generation(40),
        ..job(10, 0xa1, 0, succeeded())
    };
    let cards = [
        card(&machine("lab", vec![rollback])),
        card(&following(machine("nova", vec![]), "main", SHA)),
    ];

    assert_eq!(
        derive::deployments(&cards, NOW)[0].rows,
        vec![row("lab", Some(0), Verdict::Done, Some(10))]
    );
}
