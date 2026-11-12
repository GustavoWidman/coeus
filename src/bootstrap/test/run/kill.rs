use futures_util::future::join_all;

use super::{ANSWERS, Fleet, fake::Kill, has_nix};

/// a run dropped at each call it makes into the machine or the forge in
/// turn, as ctrl-c, kill -9 or a dead admin would leave it, and run again
#[tokio::test]
async fn a_run_stopped_anywhere_converges_when_run_again() {
    if !has_nix() {
        return;
    }
    let counted = Fleet::new().await;
    let counting = Kill::default();
    counted.assert_in_the_fleet(&counted.run(&counting).await.unwrap());
    let points = counting.seen();
    assert!(points >= 19, "only {points} places to stop at");

    join_all((0..points).map(|at| async move {
        let fleet = Fleet::new().await;
        assert!(
            fleet.run(&Kill::at(at)).await.is_none(),
            "the run went past {at} of {points}"
        );
        let ran = fleet.run(&Kill::default()).await.unwrap();
        fleet.assert_in_the_fleet(&ran);
        let world = fleet.world.lock().unwrap();
        assert_eq!(world.jobs.len(), 1, "stopped at {at}: one switch only");
    }))
    .await;
}

/// the same on a NixOS that refuses to switch live: a run dropped after the
/// refusal, while the boot runs, after it succeeded with the machine still
/// on its stock system, and after the reboot, converges when run again
#[tokio::test]
async fn a_refused_switch_stopped_anywhere_converges_when_run_again() {
    if !has_nix() {
        return;
    }
    let counted = Fleet::new().await;
    counted.world.lock().unwrap().refuses = true;
    let counting = Kill::default();
    counted.assert_in_the_fleet(&counted.run(&counting).await.unwrap());
    counted.assert_booted_into_the_fleet();
    let points = counting.seen();

    join_all((0..points).map(|at| async move {
        let fleet = Fleet::new().await;
        fleet.world.lock().unwrap().refuses = true;
        assert!(
            fleet.run(&Kill::at(at)).await.is_none(),
            "the run went past {at} of {points}"
        );
        let ran = fleet.run(&Kill::default()).await.unwrap();
        fleet.assert_in_the_fleet(&ran);
        fleet.assert_booted_into_the_fleet();
    }))
    .await;
}

/// a run dropped wherever it was waiting, inside git, nix or age as much as
/// on the machine: after its `n`-th wait for a spread of `n`. what a dropped
/// git left running is given time to end, as it would before anyone ran the
/// bootstrap again. how often a run waits depends on how busy the machine
/// is, so a run may finish before its `n`-th wait; enough of them must not.
#[tokio::test]
async fn a_run_dropped_while_it_waits_on_anything_converges_when_run_again() {
    if !has_nix() {
        return;
    }
    let counted = Fleet::new().await;
    let (ran, waits) = counted
        .run_polled(&Kill::default(), ANSWERS, false, None)
        .await;
    counted.assert_in_the_fleet(&ran.unwrap());

    const CASES: usize = 48;
    let stride = (waits / CASES).max(1);
    let stopped = join_all((1..waits).step_by(stride).map(|at| async move {
        let fleet = Fleet::new().await;
        let (finished, _) = fleet
            .run_polled(&Kill::default(), ANSWERS, false, Some(at))
            .await;
        if finished.is_some() {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let ran = fleet.run(&Kill::default()).await.unwrap();
        fleet.assert_in_the_fleet(&ran);
        let world = fleet.world.lock().unwrap();
        assert_eq!(world.jobs.len(), 1, "dropped at wait {at}: one switch only");
        true
    }))
    .await;
    let stopped = stopped.into_iter().filter(|stopped| *stopped).count();
    assert!(stopped >= CASES / 8, "only {stopped} runs were stopped");
}
