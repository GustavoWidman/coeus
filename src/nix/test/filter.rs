use super::{BUILD, DOWNLOAD, FAILING};
use crate::nix::*;

/// what a job log keeps of `log`, in the order it writes it
fn filtered(log: &str) -> Vec<String> {
    let mut filter = LogFilter::new();
    let mut kept = Vec::new();
    for raw in log.lines() {
        match filter.keep(raw, &Line::parse(raw)) {
            Kept::Nothing => continue,
            Kept::After(held) => kept.push(held),
            Kept::Line => {}
        }
        kept.push(raw.to_owned());
    }
    kept
}

fn fold<S: AsRef<str>>(lines: &[S]) -> LogFold {
    let mut fold = LogFold::new();
    lines.iter().for_each(|line| fold.feed(line.as_ref()));
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
    for log in [BUILD, DOWNLOAD, FAILING] {
        let kept = filtered(log);
        assert!(kept.iter().all(|line| log.lines().any(|raw| raw == line)));
        // progress left out until its activity stops goes in just before the stop, past
        // other activities' lines; everything else stays in order
        let mut rest = log.lines();
        assert!(
            kept.iter()
                .filter(|line| !line.contains(r#""type":105"#))
                .all(|line| rest.any(|raw| raw == line))
        );
    }
}

#[test]
fn judges_each_kind_of_line() {
    let mut filter = LogFilter::new();
    let mut keep = |raw| filter.keep(raw, &Line::parse(raw)) != Kept::Nothing;

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

#[test]
fn keeps_progress_a_whole_percent_at_a_time() {
    let mut filter = LogFilter::new();
    let mut keep = |raw: &str| filter.keep(raw, &Line::parse(raw));
    let at = |done: u64, running: u64| {
        format!(
            r#"@nix {{"action":"result","fields":[{done},1000,{running},0],"id":9,"type":105}}"#
        )
    };

    assert_eq!(keep(&at(0, 0)), Kept::Line);
    // a byte or a few is not a percent
    assert_eq!(keep(&at(3, 0)), Kept::Nothing);
    assert_eq!(keep(&at(9, 0)), Kept::Nothing);
    assert_eq!(keep(&at(10, 0)), Kept::Line);
    assert_eq!(keep(&at(15, 0)), Kept::Nothing);
    // what is running counts whatever the percent
    assert_eq!(keep(&at(15, 1)), Kept::Line);
    assert_eq!(keep(&at(998, 1)), Kept::Line);
    assert_eq!(keep(&at(999, 1)), Kept::Nothing);
    assert_eq!(keep(&at(1000, 1)), Kept::Line);

    // the newest report left out goes in before the activity stops
    assert_eq!(keep(&at(1000, 0)), Kept::Line);
    let stop = r#"@nix {"action":"stop","id":9}"#;
    assert_eq!(keep(&at(1000, 0)), Kept::Nothing);
    assert_eq!(keep(stop), Kept::Line);
    let _ = keep(&at(0, 0));
    assert_eq!(keep(&at(4, 0)), Kept::Nothing);
    assert_eq!(keep(stop), Kept::After(at(4, 0)));
}

#[test]
fn keeps_every_change_of_progress_without_a_size() {
    let mut filter = LogFilter::new();
    let mut keep = |done: u64| {
        let raw =
            format!(r#"@nix {{"action":"result","fields":[{done},0,0,0],"id":5,"type":105}}"#);
        filter.keep(&raw, &Line::parse(&raw))
    };
    assert_eq!(keep(1), Kept::Line);
    assert_eq!(keep(2), Kept::Line);
    assert_eq!(keep(2), Kept::Nothing);
}
