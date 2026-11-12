use std::{collections::VecDeque, time::Duration};

use crate::{
    common::model::{
        Action, Failure, JobId, JobState, JobSummary, OrderId, Origin, Outcome, Phase, Source,
        Timestamp,
    },
    engine::tries::may_try,
};

const OLD: &str = "1111111111111111111111111111111111111111";
const NEW: &str = "2222222222222222222222222222222222222222";
const MINUTE: Duration = Duration::from_secs(60);

fn git(rev: &str) -> Source {
    Source::Git {
        rev: rev.parse().unwrap(),
        git_ref: Some("deployed".into()),
    }
}

fn failed(phase: Phase) -> Outcome {
    Outcome::Failed(Failure::new(phase, "cannot reach github.com"))
}

/// a finished try of `rev`, `minutes` after the epoch
fn tried(rev: &str, outcome: Outcome, minutes: u64) -> JobSummary {
    JobSummary {
        id: JobId(1),
        order: OrderId(1),
        source: git(rev),
        action: Action::Switch,
        origin: Origin::Follow,
        state: JobState::Finished(outcome),
        ordered_at: Timestamp(0),
        started_at: None,
        finished_at: Some(Timestamp(0).after(MINUTE * minutes as u32)),
        toplevel: None,
        attempt: 1,
    }
}

fn at(minutes: u64) -> Timestamp {
    Timestamp(0).after(MINUTE * minutes as u32)
}

#[test]
fn a_commit_never_tried_here_may_be_ordered() {
    assert!(may_try(&VecDeque::new(), &git(NEW), at(0)));

    let other = VecDeque::from([tried(OLD, failed(Phase::Building), 0)]);
    assert!(may_try(&other, &git(NEW), at(0)));
}

#[test]
fn a_commit_that_failed_once_it_built_or_activated_waits_for_the_ref() {
    for outcome in [
        failed(Phase::Building),
        failed(Phase::Activating),
        Outcome::RolledBack(Failure::new(Phase::Verifying, "unreachable")),
        Outcome::Interrupted,
        Outcome::Cancelled,
    ] {
        let recent = VecDeque::from([tried(NEW, outcome.clone(), 0)]);
        assert!(
            !may_try(&recent, &git(NEW), at(24 * 60)),
            "{outcome:?} is tried again"
        );
    }
}

#[test]
fn a_commit_that_failed_before_building_is_tried_again_after_a_while() {
    let recent = VecDeque::from([tried(NEW, failed(Phase::Fetching), 10)]);
    assert!(!may_try(&recent, &git(NEW), at(10)));
    assert!(may_try(&recent, &git(NEW), at(11)));

    let recent = VecDeque::from([
        tried(NEW, failed(Phase::Evaluating), 12),
        tried(NEW, failed(Phase::Fetching), 10),
    ]);
    assert!(!may_try(&recent, &git(NEW), at(13)));
    assert!(may_try(&recent, &git(NEW), at(14)));
}

#[test]
fn three_failed_tries_wait_for_the_ref() {
    let recent = VecDeque::from([
        tried(NEW, failed(Phase::Fetching), 17),
        tried(NEW, failed(Phase::Fetching), 13),
        tried(NEW, failed(Phase::Fetching), 10),
    ]);
    assert!(!may_try(&recent, &git(NEW), at(24 * 60)));
}

#[test]
fn only_failures_since_the_last_success_count() {
    let recent = VecDeque::from([
        tried(NEW, failed(Phase::Fetching), 30),
        tried(NEW, Outcome::Succeeded, 20),
        tried(NEW, failed(Phase::Fetching), 13),
        tried(NEW, failed(Phase::Fetching), 10),
    ]);
    assert!(may_try(&recent, &git(NEW), at(31)));
}

#[test]
fn snapshots_and_generations_are_never_held_back() {
    let recent = VecDeque::from([tried(NEW, Outcome::Cancelled, 0)]);
    assert!(may_try(&recent, &Source::Generation(3), at(0)));
}
