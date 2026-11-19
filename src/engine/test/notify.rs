use std::{collections::VecDeque, time::Duration};

use crate::{
    common::model::{
        Action, Failure, JobId, JobState, JobSummary, OrderId, Origin, Outcome, Phase, Source,
        Timestamp,
    },
    engine::notify::catch_up,
    push::Notice,
};

const REV: &str = "3f2a9c1d0e4b8a7c6f5e4d3c2b1a09f8e7d6c5b4";
const MINUTE: Duration = Duration::from_secs(60);
const CALM: bool = false;
const URGENT: bool = true;

fn fetching() -> Outcome {
    Outcome::Failed(Failure::new(
        Phase::Fetching,
        "could not resolve github.com\nwhile fetching the flake",
    ))
}

fn building() -> Outcome {
    Outcome::Failed(Failure::new(
        Phase::Building,
        "builder for '/nix/store/5d1f…-zfs-kernel-2.3.4.drv' failed with exit code 2",
    ))
}

/// job `id`, ordered by `origin`, that ran from minute `id * 10` for four
/// minutes and ten seconds
fn job(id: u64, origin: Origin, outcome: Outcome) -> JobSummary {
    let started = Timestamp(0).after(MINUTE * 10 * id as u32);
    JobSummary {
        id: JobId(id),
        order: OrderId(id),
        source: Source::Git {
            rev: REV.parse().unwrap(),
            git_ref: Some("deployed".into()),
        },
        action: Action::Switch,
        origin,
        state: JobState::Finished(outcome),
        ordered_at: started,
        started_at: Some(started),
        finished_at: Some(started.after(Duration::from_secs(250))),
        toplevel: None,
        attempt: 1,
    }
}

fn user() -> Origin {
    Origin::User {
        user: "gustavo".into(),
        host: "altar".into(),
    }
}

/// what laptop-mac says about the newest of `jobs`, given oldest first
fn said(jobs: &[JobSummary]) -> Option<Notice> {
    let recent: VecDeque<_> = jobs.iter().rev().cloned().collect();
    catch_up(&"laptop-mac".parse().unwrap(), &recent[0], &recent)
}

fn notice(title: &str, body: &str, urgent: bool, id: u64) -> Option<Notice> {
    Some(Notice {
        title: title.into(),
        body: body.into(),
        path: format!("machines/laptop-mac/jobs/{id}"),
        tag: "catch-up/laptop-mac".into(),
        urgent,
    })
}

#[test]
fn a_catch_up_that_succeeds_always_speaks() {
    assert_eq!(
        said(&[job(1, Origin::Follow, Outcome::Succeeded)]),
        notice(
            "laptop-mac caught up to 3f2a9c1",
            "switched in 4m10s",
            CALM,
            1
        )
    );
    assert_eq!(
        said(&[job(1, Origin::Follow, Outcome::UpToDate)]),
        notice(
            "laptop-mac caught up to 3f2a9c1",
            "already ran its system",
            CALM,
            1
        )
    );
    assert_eq!(
        said(&[
            job(1, Origin::Follow, fetching()),
            job(2, Origin::Follow, Outcome::Succeeded),
        ]),
        notice(
            "laptop-mac caught up to 3f2a9c1",
            "switched in 4m10s",
            CALM,
            2
        ),
        "a success after a failed try"
    );
}

#[test]
fn a_failure_past_evaluating_ends_the_tries_and_speaks_at_once() {
    let stopped = |body: &str| notice("laptop-mac stopped catching up to 3f2a9c1", body, URGENT, 1);
    let cases = [
        (
            building(),
            "failed building: builder for '/nix/store/5d1f…-zfs-kernel-2.3.4.drv' failed with exit code 2",
        ),
        (
            Outcome::RolledBack(Failure::new(Phase::Verifying, "tailscale stayed down")),
            "rolled back after verifying: tailscale stayed down",
        ),
        (Outcome::Interrupted, "interrupted while activating"),
    ];
    for (outcome, line) in cases {
        assert_eq!(
            said(&[job(1, Origin::Follow, outcome)]),
            stopped(&format!("{line}\ntries again when deployed moves"))
        );
    }
}

#[test]
fn a_failure_before_building_speaks_only_on_the_last_try() {
    let tries: Vec<_> = (1..=3)
        .map(|id| job(id, Origin::Follow, fetching()))
        .collect();

    assert_eq!(said(&tries[..1]), None, "the first of three tries");
    assert_eq!(said(&tries[..2]), None, "the second of three tries");
    assert_eq!(
        said(&tries),
        notice(
            "laptop-mac stopped catching up to 3f2a9c1",
            "failed fetching 3 times: could not resolve github.com\ntries again when deployed moves",
            URGENT,
            3
        )
    );
}

#[test]
fn tries_count_since_the_last_success_whoever_ordered_them() {
    let after_success = [
        job(1, Origin::Follow, fetching()),
        job(2, Origin::Follow, fetching()),
        job(3, user(), Outcome::Succeeded),
        job(4, Origin::Follow, fetching()),
    ];
    assert_eq!(said(&after_success), None);

    let after_a_deploy = [
        job(1, user(), fetching()),
        job(2, user(), fetching()),
        job(3, Origin::Follow, fetching()),
    ];
    assert!(said(&after_a_deploy).is_some());
}

#[test]
fn only_jobs_the_machine_ordered_itself_speak() {
    let github = Origin::GitHub {
        repository: "GustavoWidman/nix".into(),
        workflow: "deploy".into(),
        run_id: 42,
        attempt: 1,
    };
    for origin in [user(), github] {
        for outcome in [Outcome::Succeeded, building()] {
            assert_eq!(
                said(&[job(1, origin.clone(), outcome.clone())]),
                None,
                "{origin} {outcome}"
            );
        }
    }
}

#[test]
fn a_cancelled_or_superseded_catch_up_says_nothing() {
    for outcome in [Outcome::Cancelled, Outcome::Superseded { by: JobId(2) }] {
        assert_eq!(
            said(&[job(1, Origin::Follow, outcome.clone())]),
            None,
            "{outcome}"
        );
    }
}

#[test]
fn a_long_failure_is_quoted_to_one_short_line() {
    let message = format!("{}\nsecond line", "x".repeat(200));
    let outcome = Outcome::Failed(Failure::new(Phase::Building, message));
    let body = said(&[job(1, Origin::Follow, outcome)]).unwrap().body;
    let first = body.lines().next().unwrap();
    assert_eq!(first, format!("failed building: {}…", "x".repeat(80)));
    assert!(!body.contains("second line"));
}
