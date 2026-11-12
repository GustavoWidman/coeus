use super::{BUILD, DOWNLOAD, FAILING};
use crate::nix::*;

fn filtered(log: &str) -> Vec<&str> {
    let mut filter = LogFilter::new();
    log.lines()
        .filter(|line| filter.keep(&Line::parse(line)))
        .collect()
}

fn fold(lines: &[&str]) -> LogFold {
    let mut fold = LogFold::new();
    lines.iter().for_each(|line| fold.feed(line));
    fold
}

#[test]
fn drops_chatter_and_repeated_results() {
    // `nom --json` renders each fixture the same before and after filtering
    assert_eq!(filtered(BUILD).len(), 75);
    assert_eq!(filtered(DOWNLOAD).len(), 117);
    assert_eq!(filtered(FAILING).len(), 77);
}

#[test]
fn keeps_everything_the_fold_reads() {
    for log in [BUILD, DOWNLOAD, FAILING] {
        let all: Vec<_> = log.lines().collect();
        let (all, kept) = (fold(&all), fold(&filtered(log)));

        assert_eq!(all.progress(), kept.progress());
        assert_eq!(all.evaluated(), kept.evaluated());
        assert!(all.tail().eq(kept.tail()));
        assert_eq!(all.last_error(), kept.last_error());
    }
}

#[test]
fn keeps_lines_verbatim_and_in_order() {
    let kept = filtered(FAILING);
    let mut rest = FAILING.lines();
    assert!(kept.iter().all(|line| rest.any(|raw| raw == *line)));
}

#[test]
fn judges_each_kind_of_line() {
    let mut filter = LogFilter::new();
    let mut keep = |raw| filter.keep(&Line::parse(raw));

    assert!(keep("plain output"));
    assert!(keep(
        r#"@nix {"action":"msg","level":3,"msg":"these 2 paths will be fetched"}"#
    ));
    assert!(!keep(
        r#"@nix {"action":"msg","level":4,"msg":"evaluating file 'x'"}"#
    ));
    assert!(!keep(r#"@nix {"action":"msg","level":6,"msg":"debug"}"#));

    let progress = r#"@nix {"action":"result","fields":[0,1,0,0],"id":3,"type":105}"#;
    assert!(keep(progress));
    assert!(!keep(progress));
    assert!(keep(
        r#"@nix {"action":"result","fields":[0,1,0,0],"id":4,"type":105}"#
    ));
    assert!(keep(
        r#"@nix {"action":"result","fields":[1,1,0,0],"id":3,"type":105}"#
    ));

    let downloads = r#"@nix {"action":"result","fields":[101,0],"id":2,"type":106}"#;
    let unpacked = r#"@nix {"action":"result","fields":[100,0],"id":2,"type":106}"#;
    assert!(keep(downloads));
    assert!(keep(unpacked));
    assert!(!keep(downloads));
    assert!(!keep(unpacked));

    let line = r#"@nix {"action":"result","fields":["again"],"id":7,"type":101}"#;
    assert!(keep(line));
    assert!(keep(line));

    // what a stopped activity reported is forgotten
    assert!(keep(r#"@nix {"action":"stop","id":3}"#));
    assert!(keep(progress));
}
