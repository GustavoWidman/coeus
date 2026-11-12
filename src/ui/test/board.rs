use std::time::Duration;

use super::*;
use crate::{
    client::{FleetEvent, Link},
    common::model::{Action, JobState, Outcome, Platform, Rejection},
    ui::{
        Frame,
        board::{Ordering, Status, Verdict, eta},
        text,
    },
};

fn frame(width: usize) -> Frame {
    Frame {
        now: NOW,
        width,
        height: 40,
    }
}

#[test]
fn a_building_row_shows_progress_counts_and_the_active_build() {
    let mut board = order_board(&["altar", "lab"]);
    admit(&mut board, "lab", 12);
    show(&mut board, view("lab", Some(building(12)), Vec::new()));

    let lines = plain(&board.lines(frame(160)));
    let row = lines.iter().find(|line| line.contains("lab")).unwrap();

    assert!(
        lines[0].starts_with("coeus switch main@caf4e2b → 2 machines"),
        "{}",
        lines[0]
    );
    assert!(row.contains("building"), "{row}");
    assert!(row.contains("58%"), "{row}");
    assert!(
        row.contains("142/225 built · 12/40 fetched · 1.2/3.4 GiB"),
        "{row}"
    );
    assert!(row.contains("firefox-121.0 +1"), "{row}");
    assert!(row.contains("9m59s"), "{row}");
    assert!(
        lines
            .last()
            .unwrap()
            .contains("ctrl-c detaches, deployments keep running")
    );
}

#[test]
fn every_line_fits_the_terminal_width() {
    let mut board = order_board(&["altar", "lab"]);
    admit(&mut board, "lab", 12);
    show(&mut board, view("lab", Some(building(12)), Vec::new()));

    for width in [30, 60, 100] {
        for line in board.lines(frame(width)) {
            assert!(text::width(&line) <= width, "{width}: {line}");
        }
    }
}

#[test]
fn failures_name_their_reason_and_how_to_look_closer() {
    let mut board = order_board(&["lab"]);
    admit(&mut board, "lab", 12);
    show(&mut board, view("lab", None, vec![failed(12)]));

    let lines = plain(&board.lines(frame(200)));
    assert!(lines[2].starts_with("✗ lab  failed"), "{}", lines[2]);
    assert!(
        lines[2].contains("in building: builder for 'hello.drv' failed"),
        "{}",
        lines[2]
    );
    assert!(lines[2].contains("coeus attach lab #12"), "{}", lines[2]);
}

#[test]
fn rows_beyond_the_terminal_height_collapse() {
    let names: Vec<String> = (0..30).map(|at| format!("host-{at}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let board = watch_board(&names);

    let lines = board.lines(Frame {
        now: NOW,
        width: 80,
        height: 15,
    });
    assert!(lines.len() < 15);
    assert!(
        plain(&lines)
            .iter()
            .any(|line| line.contains("… 21 more machines"))
    );
}

#[test]
fn an_ordered_row_follows_its_own_job() {
    let mut board = order_board(&["lab"]);
    let lab = host("lab");
    assert!(matches!(
        board.row(&lab).unwrap().status(),
        Status::Pending { wave: 1 }
    ));

    admit(&mut board, "lab", 12);
    assert_eq!(
        board.row(&lab).unwrap().status(),
        Status::Admitted(JobId(12))
    );

    // another job runs; ours finished
    let done = summary(12, JobState::Finished(Outcome::Succeeded));
    show(&mut board, view("lab", Some(building(13)), vec![done]));
    let row = board.row(&lab).unwrap();
    assert!(matches!(row.status(), Status::Finished(summary, _) if summary.id == JobId(12)));
    assert_eq!(row.verdict(), Some(Verdict::Done));
    assert!(board.is_settled());
}

#[test]
fn verdicts_cover_refusals_and_silence() {
    let mut board = order_board(&["altar", "lab"]);
    board.order(
        &host("altar"),
        Ordering::Rejected(Rejection::Unsupported {
            action: Action::Boot,
            platform: Platform::Darwin,
        }),
    );
    board.order(
        &host("lab"),
        Ordering::Unreachable("connection refused".into()),
    );

    let verdicts: Vec<Verdict> = board.verdicts().collect();
    assert_eq!(verdicts, [Verdict::Rejected, Verdict::Unreachable]);

    let lines = plain(&board.lines(frame(200)));
    assert!(lines[2].contains("boot is not available on darwin"));
}

#[test]
fn a_watched_row_shows_whatever_runs_and_never_settles() {
    let mut board = watch_board(&["lab"]);
    assert_eq!(
        board.row(&host("lab")).unwrap().status(),
        Status::Connecting
    );

    show(&mut board, view("lab", Some(building(3)), Vec::new()));
    assert!(matches!(
        board.row(&host("lab")).unwrap().status(),
        Status::Running(_)
    ));
    assert!(!board.is_settled());

    let row = plain(&board.lines(frame(200)))[2].clone();
    assert!(row.contains("#3 main@caf4e2b by r3dlust@altar"), "{row}");
}

#[test]
fn a_dropped_connection_shows_reconnecting_until_the_job_ends() {
    let mut board = watch_board(&["lab"]);
    show(&mut board, view("lab", Some(building(3)), Vec::new()));
    board.apply(FleetEvent::Link {
        host: host("lab"),
        link: Link::Connecting,
    });

    let row = board.row(&host("lab")).unwrap();
    assert!(row.is_reconnecting());
    assert!(plain(&board.lines(frame(200)))[2].starts_with('↻'));
}

#[test]
fn the_eta_comes_from_recent_jobs_or_the_build_rate() {
    let job = building(12);

    let mut quick = summary(8, JobState::Finished(Outcome::Succeeded));
    quick.finished_at = Some(Timestamp(ORDERED.0 + 1_000 + 900_000));
    let machine = view("lab", None, vec![quick]);
    // a fifteen-minute job, ten minutes in
    assert_eq!(
        eta(&machine, &job, NOW),
        Some(Duration::from_millis(900_000 - 599_000))
    );

    // nine minutes of building for 58% of the work
    let alone = view("lab", None, Vec::new());
    let left = eta(&alone, &job, NOW).unwrap().as_secs();
    assert!((385..=395).contains(&left), "{left}");
}
