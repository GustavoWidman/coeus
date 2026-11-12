use super::{BUILD, DOWNLOAD, FAILING};
use crate::{
    common::model::{Phase, Tally},
    nix::*,
};

fn folded(log: &str) -> LogFold {
    let mut fold = LogFold::new();
    log.lines().for_each(|line| fold.feed(line));
    fold
}

/// the fold after every line up to and including the first one holding `marker`
fn folded_until(log: &str, marker: &str) -> LogFold {
    let mut fold = LogFold::new();
    for line in log.lines() {
        fold.feed(line);
        if line.contains(marker) {
            return fold;
        }
    }
    panic!("no line holds {marker}");
}

const fn tally(done: u32, expected: u32, running: u32, failed: u32) -> Tally {
    Tally {
        done,
        expected,
        running,
        failed,
    }
}

#[test]
fn counts_a_local_build() {
    let fold = folded(BUILD);
    let progress = fold.progress();

    assert_eq!(progress.builds, tally(1, 2, 0, 0));
    assert!(progress.downloads.is_empty());
    assert_eq!(progress.downloaded_bytes, 0);
    assert!(progress.active.is_empty());
    assert!(fold.evaluated());
}

#[test]
fn counts_downloads_and_their_bytes() {
    let progress = folded(DOWNLOAD).progress().clone();

    assert_eq!(progress.downloads, tally(2, 2, 0, 0));
    assert!(progress.builds.is_empty());
    // the two NAR downloads; the narinfo lookups before them are not counted
    assert_eq!(progress.downloaded_bytes, 1_292_513 + 31_066);
    assert_eq!(progress.expected_bytes, 1_323_579);
    assert_eq!(progress.fraction(), Some(1.0));
}

#[test]
fn bytes_grow_while_a_path_downloads() {
    let fold = folded_until(DOWNLOAD, r#""fields":[782810,1292513,0,0]"#);
    assert_eq!(fold.progress().downloaded_bytes, 782_810);
    assert_eq!(fold.progress().downloads, tally(0, 2, 1, 0));
}

#[test]
fn evaluation_ends_when_nix_starts_counting_builds() {
    assert!(!folded_until(BUILD, "this derivation will be built").evaluated());
    assert!(folded_until(BUILD, r#""type":104"#).evaluated());
}

#[test]
fn names_the_running_builds_oldest_first() {
    let both = folded_until(
        FAILING,
        "building '/nix/store/r88bdlsl5383rv6amgn6idmy9wv0k5r7",
    );
    assert_eq!(
        both.progress().active,
        ["coeus-count-1794500001", "coeus-greet-1794500001"]
    );

    let last = folded_until(
        FAILING,
        "building '/nix/store/g1w6lisiqps834vb8k4nkvlkihfsrr4b",
    );
    assert_eq!(last.progress().active, ["coeus-broken-1794500001"]);
}

#[test]
fn tails_the_failing_build() {
    let fold = folded_until(FAILING, "fatal: something went wrong");
    let tail: Vec<_> = fold.tail().collect();

    assert_eq!(tail.len(), 20);
    assert_eq!(tail[0], "coeus-broken-1794500001> broken line 7");
    assert_eq!(
        tail[19],
        "coeus-broken-1794500001> fatal: something went wrong"
    );
}

#[test]
fn explains_a_failure_with_the_last_error() {
    let fold = folded(FAILING);
    assert_eq!(fold.progress().builds, tally(2, 3, 0, 1));

    let failure = fold.failure(Phase::Building);
    assert_eq!(failure.phase, Phase::Building);
    assert_eq!(
        failure.message,
        "Cannot build '/nix/store/g1w6lisiqps834vb8k4nkvlkihfsrr4b-coeus-broken-1794500001.drv'."
    );
    assert_eq!(failure.tail.len(), 20);
    assert!(failure.tail.iter().all(|line| !line.contains('\u{1b}')));
    assert!(
        failure
            .tail
            .iter()
            .any(|line| line.ends_with("> fatal: something went wrong"))
    );
    assert_eq!(
        failure.tail.last().map(|line| line.trim()),
        Some("nix log /nix/store/g1w6lisiqps834vb8k4nkvlkihfsrr4b-coeus-broken-1794500001.drv")
    );
}

#[test]
fn explains_an_error_with_a_trace_by_its_cause() {
    let mut fold = LogFold::new();
    fold.feed(concat!(
        r#"@nix {"action":"msg","level":0,"msg":"\u001b[31;1merror:\u001b[0m\n"#,
        r#"       … while fetching the input '\u001b[35;1mgit+https://github.com/owner/nix?rev=5d93c5b\u001b[0m'\n\n"#,
        r#"       \u001b[31;1merror:\u001b[0m Failed to fetch git repository '\u001b[35;1mhttps://github.com/owner/nix\u001b[0m'"}"#,
    ));

    assert_eq!(
        fold.last_error(),
        Some("Failed to fetch git repository 'https://github.com/owner/nix'")
    );
}

#[test]
fn keeps_plain_output_in_the_tail() {
    let mut fold = LogFold::new();
    fold.feed("activating the configuration...\n");
    fold.feed("\u{1b}[1;31mswitch failed\u{1b}[0m\n");
    fold.feed("   \n");

    assert_eq!(
        fold.tail().collect::<Vec<_>>(),
        ["activating the configuration...", "switch failed"]
    );
    assert_eq!(fold.failure(Phase::Activating).message, "nix failed");
}

#[test]
fn snapshots_only_when_progress_changed() {
    let mut fold = folded(BUILD);
    assert_eq!(fold.snapshot().as_ref(), Some(fold.progress()));
    assert_eq!(fold.snapshot(), None);

    // the Builds aggregate repeating its last count
    let builds = BUILD
        .lines()
        .rfind(|line| line.contains(r#""type":105}"#) && line.contains("[1,2,0,0]"))
        .unwrap();
    let mut fold = folded_until(BUILD, builds);
    fold.snapshot();
    fold.feed(builds);
    assert_eq!(fold.snapshot(), None);
}
