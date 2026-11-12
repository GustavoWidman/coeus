use crate::common::model::{ClosureDiff, PackageChange};

const GIT: &str = include_str!("../fixtures/closures-git.txt");
const SYSTEM: &str = include_str!("../fixtures/closures-system.txt");
const ROLLBACK: &str = include_str!("../fixtures/closures-rollback.txt");

fn change(name: &str, before: &[&str], after: &[&str], size_delta: i64) -> PackageChange {
    PackageChange {
        name: name.to_owned(),
        before: before.iter().map(|version| (*version).to_owned()).collect(),
        after: after.iter().map(|version| (*version).to_owned()).collect(),
        size_delta,
    }
}

fn find<'a>(diff: &'a ClosureDiff, name: &str) -> &'a PackageChange {
    diff.changes
        .iter()
        .find(|change| change.name == name)
        .unwrap_or_else(|| panic!("no change to {name}"))
}

#[test]
fn reads_every_line_of_real_output() {
    for output in [GIT, SYSTEM, ROLLBACK] {
        assert_eq!(
            ClosureDiff::parse(output).changes.len(),
            output.lines().count()
        );
    }
}

#[test]
fn reads_upgrades_with_and_without_size() {
    let diff = ClosureDiff::parse(GIT);

    assert_eq!(
        find(&diff, "expat"),
        &change("expat", &["2.8.5"], &["2.9.0"], 0)
    );
    assert_eq!(
        find(&diff, "git"),
        &change("git", &["2.55.0"], &["2.56.0"], 1_572_864)
    );
    assert_eq!(
        find(&diff, "gzip"),
        &change("gzip", &["1.14"], &["1.15"], -15_565)
    );
}

#[test]
fn reads_removals_and_size_only_changes() {
    let diff = ClosureDiff::parse(GIT);

    let bash = find(&diff, "bash-interactive");
    assert!(bash.is_removed());
    assert_eq!(
        bash,
        &change("bash-interactive", &["5.3p15"], &[], -8_493_466)
    );

    let clang = find(&diff, "clang");
    assert_eq!(clang, &change("clang", &[], &[], -114_790));
    assert!(!clang.is_added() && !clang.is_removed());
}

#[test]
fn reads_additions_of_several_versions() {
    let diff = ClosureDiff::parse(SYSTEM);

    assert_eq!(
        find(&diff, "ffmpeg"),
        &change("ffmpeg", &[], &["8.1.2", "9.0.2"], 57_881_395)
    );
    assert_eq!(
        find(&diff, "brew"),
        &change("brew", &[], &["", "7.0.11"], 20_552_090)
    );
    let brewfile = find(&diff, "Brewfile");
    assert!(brewfile.is_added());
    assert_eq!(brewfile, &change("Brewfile", &[], &[""], 0));
    assert_eq!(find(&diff, "brotli").size_delta, 1_992_294);
}

#[test]
fn sums_the_size_delta() {
    let diff = ClosureDiff::parse(ROLLBACK);
    assert_eq!(
        diff.changes,
        [
            change("disable-launchd", &[""], &[], 0),
            change("source", &[], &[], -1_782_579),
        ]
    );
    assert_eq!(diff.size_delta(), -1_782_579);
}

#[test]
fn reads_the_older_format() {
    let diff = ClosureDiff::parse(
        "dolphin: 20.08.1 → 20.08.2, +13.9 KiB\n\
         kdeconnect: 20.08.2 removed, -6597.8 KiB\n\
         libfoo: 1.2, 1.3 → 1.4\n\
         hello: ∅ → ε\n\
         bluez-qt: +12.6 KiB\n",
    );

    assert_eq!(
        diff.changes,
        [
            change("dolphin", &["20.08.1"], &["20.08.2"], 14_234),
            change("kdeconnect", &["20.08.2"], &[], -6_756_147),
            change("libfoo", &["1.2", "1.3"], &["1.4"], 0),
            change("hello", &[], &[""], 0),
            change("bluez-qt", &[], &[], 12_902),
        ]
    );
}

#[test]
fn skips_what_it_does_not_recognise() {
    let diff = ClosureDiff::parse("warning: something\n\nhello: who knows\n");
    assert!(diff.is_empty());
}
