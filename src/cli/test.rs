use std::num::NonZeroUsize;

use crate::{
    cli::{
        exit::Exit,
        rollout::{Rollout, Step},
    },
    common::model::{Hostname, JobState, Outcome},
    ui::{
        Board,
        board::{Ordering, Verdict},
        test::{admit, order_board, show, summary, view},
    },
};

fn hosts(names: &[&str]) -> Vec<Hostname> {
    names.iter().map(|name| name.parse().unwrap()).collect()
}

/// settles `name` the way a machine would: admitted, then finished
fn finish(board: &mut Board, name: &str, outcome: Outcome) {
    admit(board, name, 12);
    show(
        board,
        view(name, None, vec![summary(12, JobState::Finished(outcome))]),
    );
}

#[test]
fn exit_codes_follow_the_ci_contract() {
    use Verdict::*;
    assert_eq!(Exit::of([Done, Done], false), Exit::Success);
    assert_eq!(Exit::of([Done, Failed, Unreachable], false), Exit::Failed);
    assert_eq!(Exit::of([Done, Unreachable], false), Exit::Unreached);
    assert_eq!(Exit::of([Done, Unreachable], true), Exit::Success);
    assert_eq!(Exit::of([Rejected], true), Exit::Unreached);
    assert_eq!(Exit::of([Held], false), Exit::Failed);
    assert_eq!(
        [Exit::Success, Exit::Failed, Exit::Unreached].map(Exit::code),
        [0, 1, 2]
    );
}

#[test]
fn waves_put_canaries_first_then_batches() {
    let all = hosts(&["altar", "lab", "oracle-1", "oracle-2", "oracle-3"]);
    let rollout = Rollout::plan(&all, &hosts(&["lab"]), NonZeroUsize::new(2), 0, false).unwrap();

    assert_eq!(
        rollout.waves(),
        [
            hosts(&["lab"]),
            hosts(&["altar", "oracle-1"]),
            hosts(&["oracle-2", "oracle-3"]),
        ]
    );

    let single = Rollout::plan(&all, &[], None, 0, false).unwrap();
    assert_eq!(single.waves(), std::slice::from_ref(&all));
    assert!(Rollout::plan(&all, &hosts(&["desktop"]), None, 0, false).is_err());
}

#[test]
fn the_next_wave_waits_for_the_last_and_stops_past_the_budget() {
    let names = ["lab", "altar", "oracle"];
    let mut board = order_board(&names);
    let mut rollout = Rollout::plan(
        &hosts(&names),
        &hosts(&["lab"]),
        NonZeroUsize::new(1),
        0,
        false,
    )
    .unwrap();

    assert_eq!(
        rollout.advance(&board),
        Step::Order {
            wave: 1,
            hosts: hosts(&["lab"])
        }
    );
    assert_eq!(rollout.advance(&board), Step::Wait);

    finish(&mut board, "lab", Outcome::Succeeded);
    assert_eq!(
        rollout.advance(&board),
        Step::Order {
            wave: 2,
            hosts: hosts(&["altar"])
        }
    );

    finish(&mut board, "altar", Outcome::Interrupted);
    assert_eq!(rollout.advance(&board), Step::Stop(hosts(&["oracle"])));

    board.order(&"oracle".parse().unwrap(), Ordering::Held);
    assert_eq!(rollout.advance(&board), Step::Done);
    assert!(board.is_settled());
}

#[test]
fn a_failure_budget_lets_the_rollout_continue() {
    let names = ["lab", "altar"];
    let mut board = order_board(&names);
    let mut rollout = Rollout::plan(&hosts(&names), &hosts(&["lab"]), None, 1, false).unwrap();

    rollout.advance(&board);
    finish(&mut board, "lab", Outcome::Cancelled);
    assert_eq!(
        rollout.advance(&board),
        Step::Order {
            wave: 2,
            hosts: hosts(&["altar"])
        }
    );
}
