use super::*;
use crate::{
    client::{FleetEvent, Link},
    common::model::{JobState, Outcome, Timestamp},
    ui::Plain,
};

fn later(seconds: i64) -> Timestamp {
    Timestamp(NOW.0 + seconds * 1_000)
}

#[test]
fn prints_a_line_per_change_and_nothing_in_between() {
    let mut board = order_board(&["lab"]);
    let mut reporter = Plain::default();

    let waiting = plain(&reporter.observe(&board, NOW));
    assert_eq!(waiting.len(), 1);
    assert!(waiting[0].ends_with("lab  waits for wave 1"), "{waiting:?}");
    assert!(reporter.observe(&board, NOW).is_empty());

    admit(&mut board, "lab", 12);
    let ordered = plain(&reporter.observe(&board, NOW));
    assert!(ordered[0].ends_with("ordered as job #12"), "{ordered:?}");

    show(&mut board, view("lab", Some(building(12)), Vec::new()));
    let started = plain(&reporter.observe(&board, NOW));
    assert!(
        started[0].contains("building job #12 (main@caf4e2b by r3dlust@altar)"),
        "{started:?}"
    );
    assert!(
        started[1].contains("building 58% · 142/225 built"),
        "{started:?}"
    );
    assert!(reporter.observe(&board, later(5)).is_empty());

    let done = summary(12, JobState::Finished(Outcome::Succeeded));
    show(&mut board, view("lab", None, vec![done]));
    let finished = plain(&reporter.observe(&board, later(6)));
    assert!(
        finished[0].contains("✓ succeeded in 4m00s job #12"),
        "{finished:?}"
    );
}

#[test]
fn throttles_progress_to_tenths_or_half_minutes() {
    let mut board = watch_board(&["lab"]);
    let mut reporter = Plain::default();
    let mut job = building(12);
    show(&mut board, view("lab", Some(job.clone()), Vec::new()));
    reporter.observe(&board, NOW);

    job.progress.builds.done += 1;
    show(&mut board, view("lab", Some(job.clone()), Vec::new()));
    assert!(
        reporter.observe(&board, later(10)).is_empty(),
        "same tenth, too soon"
    );
    assert_eq!(
        reporter.observe(&board, later(31)).len(),
        1,
        "half a minute passed"
    );

    job.progress.builds.done = 200;
    show(&mut board, view("lab", Some(job), Vec::new()));
    let next = plain(&reporter.observe(&board, later(32)));
    assert!(next[0].contains("building 80%"), "{next:?}");
}

#[test]
fn failures_print_their_tail_and_reconnects_are_announced() {
    let mut board = watch_board(&["lab"]);
    let mut reporter = Plain::default();
    show(&mut board, view("lab", Some(building(12)), Vec::new()));
    reporter.observe(&board, NOW);

    board.apply(FleetEvent::Link {
        host: host("lab"),
        link: Link::Connecting,
    });
    let lost = plain(&reporter.observe(&board, NOW));
    assert!(
        lost[0].ends_with("lost the connection, reconnecting"),
        "{lost:?}"
    );

    show(&mut board, view("lab", None, vec![failed(12)]));
    let lines = plain(&reporter.observe(&board, NOW));
    assert!(lines[0].contains("✗ failed in 4m00s"), "{lines:?}");
    assert!(lines[1].ends_with("in building: builder for 'hello.drv' failed with exit code 1"));
    assert!(lines[2].ends_with("│ make: *** [all] Error 1"));
    assert!(lines[3].ends_with("reconnected"));
}
