use super::*;
use crate::{
    common::model::{JobState, Outcome},
    ui::{Summary, board::Ordering},
};

fn settled() -> Board {
    let mut board = order_board(&["altar", "lab", "oracle"]);
    admit(&mut board, "altar", 3);
    admit(&mut board, "lab", 12);
    board.order(
        &host("oracle"),
        Ordering::Unreachable("no answer within 5s".into()),
    );
    show(
        &mut board,
        view(
            "altar",
            None,
            vec![summary(3, JobState::Finished(Outcome::Succeeded))],
        ),
    );
    let mut lab = failed(12);
    if let JobState::Finished(Outcome::Failed(failure)) = &mut lab.state {
        failure.message = "a | b".into();
    }
    show(&mut board, view("lab", None, vec![lab]));
    board
}

#[test]
fn the_terminal_summary_tables_every_machine_and_a_verdict() {
    let board = settled();
    let lines = plain(&Summary::new(&board, NOW).lines());

    assert!(lines[0].starts_with("HOST    OUTCOME"), "{}", lines[0]);
    assert!(lines[1].starts_with("altar   succeeded"), "{}", lines[1]);
    assert!(
        lines[1].contains("main@caf4e2b  4m00s  coeus attach altar #3"),
        "{}",
        lines[1]
    );
    assert!(
        lines[3].contains("unreachable: no answer within 5s"),
        "{}",
        lines[3]
    );
    assert_eq!(
        lines.last().unwrap(),
        "✗ 1 of 3 done in 10m00s, 1 failed, 1 unreachable"
    );
}

#[test]
fn the_step_summary_is_a_markdown_table() {
    let board = settled();
    let markdown = Summary::new(&board, NOW).markdown();
    let lines: Vec<&str> = markdown.lines().collect();

    assert_eq!(lines[0], "### coeus switch `main@caf4e2b`");
    assert_eq!(lines[2], "| Host | Outcome | Source | Duration | Details |");
    assert_eq!(
        lines[4],
        "| altar | ✅ succeeded | `main@caf4e2b` | 4m00s | `coeus attach altar #3` |"
    );
    assert_eq!(
        lines[5],
        "| lab | ❌ failed in building: a \\| b | `main@caf4e2b` | 4m00s | `coeus attach lab #12` |"
    );
    assert!(
        lines[6].starts_with("| oracle | ❌ unreachable: no answer within 5s |"),
        "{}",
        lines[6]
    );
}
