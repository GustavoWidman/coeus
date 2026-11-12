use std::time::Duration;

use crate::{
    common::model::{
        Action, CancelReport, JobId, JobSelector, JobState, MachineView, NixOptions, Order,
        OrderId, Origin, Outcome, Phase, Placement, Source,
    },
    testing::{Harness, TOPLEVEL},
};

fn order(id: u64) -> Order {
    Order {
        id: OrderId(id),
        source: Source::Git {
            rev: "caf4e2b9a1d6c3e8f0b2a4c6d8e0f1a3b5c7d9e1".parse().unwrap(),
            git_ref: Some("main".into()),
        },
        action: Action::Switch,
        options: NixOptions::default(),
        origin: Origin::Follow,
        now: false,
    }
}

fn finished(view: &MachineView, id: u64) -> Option<Outcome> {
    view.recent
        .iter()
        .find(|job| job.id == JobId(id))
        .and_then(|job| match &job.state {
            JobState::Finished(outcome) => Some(outcome.clone()),
            _ => None,
        })
}

#[tokio::test]
async fn runs_an_order_to_completion() {
    let harness = Harness::start(tempfile::tempdir().unwrap()).await;

    let admission = harness.engine.deploy(order(1)).await.unwrap().unwrap();
    assert_eq!(admission.placement, Placement::Started);

    let view = harness
        .until(|view| {
            view.running
                .as_ref()
                .is_some_and(|job| job.summary.state.phase() == Some(Phase::Building))
        })
        .await;
    assert_eq!(
        view.running.unwrap().summary.toplevel.unwrap().as_str(),
        TOPLEVEL
    );

    harness.release.notify_one();
    let view = harness.until(|view| finished(view, 1).is_some()).await;
    assert_eq!(finished(&view, 1), Some(Outcome::Succeeded));
    assert!(view.running.is_none());
    harness.shutdown.cancel();
}

#[tokio::test]
async fn the_newest_order_replaces_the_waiting_one_and_duplicates_are_recognized() {
    let harness = Harness::start(tempfile::tempdir().unwrap()).await;
    harness.engine.deploy(order(1)).await.unwrap().unwrap();
    harness.engine.deploy(order(2)).await.unwrap().unwrap();
    let third = harness.engine.deploy(order(3)).await.unwrap().unwrap();

    assert_eq!(
        third.placement,
        Placement::Queued {
            behind: JobId(1),
            replaced: Some(JobId(2))
        }
    );
    assert_eq!(
        harness
            .engine
            .deploy(order(3))
            .await
            .unwrap()
            .unwrap()
            .placement,
        Placement::Duplicate
    );

    let view = harness.until(|view| finished(view, 2).is_some()).await;
    assert_eq!(
        finished(&view, 2),
        Some(Outcome::Superseded { by: JobId(3) })
    );

    harness.release.notify_one();
    harness.until(|view| finished(view, 1).is_some()).await;
    harness.release.notify_one();
    let view = harness.until(|view| finished(view, 3).is_some()).await;
    assert_eq!(finished(&view, 3), Some(Outcome::Succeeded));
    harness.shutdown.cancel();
}

#[tokio::test]
async fn cancels_waiting_and_running_jobs() {
    let harness = Harness::start(tempfile::tempdir().unwrap()).await;
    harness.engine.deploy(order(1)).await.unwrap().unwrap();
    harness.engine.deploy(order(2)).await.unwrap().unwrap();

    assert_eq!(
        harness
            .engine
            .cancel(JobSelector::Id(JobId(2)))
            .await
            .unwrap(),
        CancelReport::Withdrawn(JobId(2))
    );
    assert_eq!(
        harness.engine.cancel(JobSelector::Current).await.unwrap(),
        CancelReport::Stopping(JobId(1))
    );

    harness.release.notify_one();
    let view = harness.until(|view| finished(view, 1).is_some()).await;
    assert_eq!(finished(&view, 1), Some(Outcome::Cancelled));
    assert_eq!(finished(&view, 2), Some(Outcome::Cancelled));
    harness.shutdown.cancel();
}

#[tokio::test]
async fn rejects_what_the_platform_cannot_do() {
    let harness = Harness::start(tempfile::tempdir().unwrap()).await;
    let mut boot = order(1);
    boot.action = Action::Boot;

    assert!(harness.engine.deploy(boot).await.unwrap().is_err());
    harness.shutdown.cancel();
}

#[tokio::test]
async fn picks_up_the_jobs_left_on_disk() {
    let state = tempfile::tempdir().unwrap();
    {
        let harness = Harness::start(tempfile::TempDir::new_in(state.path()).unwrap()).await;
        drop(harness);
    }

    let first = Harness::start(state).await;
    first.engine.deploy(order(1)).await.unwrap().unwrap();
    first.engine.deploy(order(2)).await.unwrap().unwrap();
    first.until(|view| view.next.is_some()).await;
    first.shutdown.cancel();
    first.release.notify_one();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let second = Harness::start_with(first.state, first.release.clone()).await;
    let view = second
        .until(|view| finished(view, 1).is_some() && view.running.is_some())
        .await;
    assert_eq!(finished(&view, 1), Some(Outcome::Succeeded));
    assert_eq!(view.running.unwrap().summary.id, JobId(2));

    second.release.notify_one();
    second.until(|view| finished(view, 2).is_some()).await;
    second.shutdown.cancel();
}
