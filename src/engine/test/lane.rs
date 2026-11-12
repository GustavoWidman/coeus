use crate::{
    common::model::{JobId, Phase, Placement},
    engine::lane::Lane,
};

#[test]
fn an_idle_machine_starts_the_job() {
    let mut lane = Lane::Idle;
    let admitted = lane.admit(JobId(1), false, None);

    assert_eq!(admitted.placement, Placement::Started);
    assert_eq!(admitted.start, Some(JobId(1)));
    assert_eq!(lane.running(), Some(JobId(1)));
}

#[test]
fn the_newest_order_replaces_the_waiting_one() {
    let mut lane = Lane::Idle;
    lane.admit(JobId(1), false, None);
    lane.admit(JobId(2), false, Some(Phase::Building));
    let admitted = lane.admit(JobId(3), false, Some(Phase::Building));

    assert_eq!(
        admitted.placement,
        Placement::Queued {
            behind: JobId(1),
            replaced: Some(JobId(2))
        }
    );
    assert_eq!(admitted.replaced, Some(JobId(2)));
    assert_eq!(admitted.preempt, None);
    assert_eq!(lane.next(), Some(JobId(3)));
}

#[test]
fn now_preempts_only_before_activation() {
    let mut lane = Lane::Idle;
    lane.admit(JobId(1), false, None);
    let admitted = lane.admit(JobId(2), true, Some(Phase::Building));
    assert_eq!(admitted.preempt, Some(JobId(1)));
    assert_eq!(
        admitted.placement,
        Placement::Preempted { replaced: JobId(1) }
    );

    let mut lane = Lane::Idle;
    lane.admit(JobId(1), false, None);
    let admitted = lane.admit(JobId(2), true, Some(Phase::Activating));
    assert_eq!(admitted.preempt, None);
    assert!(matches!(admitted.placement, Placement::Queued { .. }));
}

#[test]
fn finishing_runs_the_waiting_job_and_withdrawing_drops_it() {
    let mut lane = Lane::Idle;
    lane.admit(JobId(1), false, None);
    lane.admit(JobId(2), false, Some(Phase::Building));

    assert_eq!(
        lane.finish(JobId(9)),
        None,
        "only the running job can finish"
    );
    assert_eq!(lane.finish(JobId(1)), Some(JobId(2)));
    assert_eq!(lane.running(), Some(JobId(2)));

    lane.admit(JobId(3), false, Some(Phase::Building));
    assert!(lane.withdraw(JobId(3)));
    assert_eq!(lane.finish(JobId(2)), None);
    assert_eq!(lane, Lane::Idle);
}
