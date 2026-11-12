use crate::{
    common::model::{JobId, JobState, Outcome, Phase, StorePath},
    engine::reconcile::{Observed, Plan, Resume},
    job::StopRequest,
};

const BUILT: &str = "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11";
const OTHER: &str = "/nix/store/9rfvhi1nqszqmny102jqxp2vmb6n0mk7-nixos-system-lab-26.10";

fn running(id: u64, phase: Phase, held: bool) -> Observed {
    Observed {
        id: JobId(id),
        state: JobState::Running { phase },
        held,
        stop: None,
        toplevel: Some(BUILT.parse().unwrap()),
    }
}

fn queued(id: u64) -> Observed {
    Observed {
        id: JobId(id),
        state: JobState::Queued,
        held: false,
        stop: None,
        toplevel: None,
    }
}

fn path(value: &str) -> StorePath {
    value.parse().unwrap()
}

#[test]
fn a_live_runner_is_followed() {
    let plan = Plan::reconcile(vec![running(1, Phase::Activating, true)], None);
    assert_eq!(plan.running, Some((JobId(1), Resume::Follow)));
    assert!(plan.close.is_empty());
}

#[test]
fn a_runner_that_died_before_activating_runs_again() {
    let plan = Plan::reconcile(vec![running(1, Phase::Building, false)], None);
    assert_eq!(plan.running, Some((JobId(1), Resume::Spawn)));
}

#[test]
fn a_dead_activation_succeeded_only_if_the_machine_runs_its_system() {
    let converged = Plan::reconcile(vec![running(1, Phase::Activating, false)], Some(&path(BUILT)));
    assert_eq!(converged.close, [(JobId(1), Outcome::Succeeded)]);
    assert_eq!(converged.running, None);

    let diverged = Plan::reconcile(vec![running(1, Phase::Verifying, false)], Some(&path(OTHER)));
    assert_eq!(diverged.close, [(JobId(1), Outcome::Interrupted)]);
}

#[test]
fn a_stop_request_closes_a_dead_runner_as_requested() {
    let mut job = running(1, Phase::Building, false);
    job.stop = Some(StopRequest::Superseded { by: JobId(2) });

    let plan = Plan::reconcile(vec![job, queued(2)], None);
    assert_eq!(plan.close, [(JobId(1), Outcome::Superseded { by: JobId(2) })]);
    assert_eq!(plan.running, Some((JobId(2), Resume::Spawn)));
}

#[test]
fn only_the_newest_queued_job_survives() {
    let plan = Plan::reconcile(
        vec![running(1, Phase::Building, true), queued(2), queued(3)],
        None,
    );

    assert_eq!(plan.running, Some((JobId(1), Resume::Follow)));
    assert_eq!(plan.next, Some(JobId(3)));
    assert_eq!(plan.close, [(JobId(2), Outcome::Superseded { by: JobId(3) })]);
}
