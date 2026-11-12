use super::*;
use crate::{
    client::{FleetEvent, Link},
    common::model::{
        ClosureDiff, FdaStatus, Follow, Generation, JobState, Outcome, PackageChange, Pinned,
    },
    ui::{diff, history, status},
};

#[test]
fn the_status_table_marks_what_needs_a_person() {
    let mut board = watch_board(&["altar", "lab", "oracle"]);

    let mut altar = view(
        "altar",
        None,
        vec![summary(3, JobState::Finished(Outcome::Succeeded))],
    );
    altar.system.fda = FdaStatus::Missing;
    altar.system.follow = Some(Follow {
        git_ref: "deployed".into(),
        latest: None,
        paused: true,
    });
    show(&mut board, altar);

    let mut lab = view("lab", Some(building(12)), Vec::new());
    lab.system.reboot_required = true;
    lab.system.pending_boot = true;
    lab.system.follow = Some(Follow {
        git_ref: "deployed".into(),
        latest: Some(Pinned {
            git_ref: "deployed".into(),
            rev: SHA.parse().unwrap(),
            committed_at: ORDERED,
        }),
        paused: false,
    });
    show(&mut board, lab);

    board.apply(FleetEvent::Link {
        host: host("oracle"),
        link: Link::Unreachable("connection refused".into()),
    });

    let lines = plain(&status::lines(&board, NOW));
    let text = lines.join("\n");

    assert!(lines[0].starts_with("HOST    STATE"), "{text}");
    assert!(
        lines[1].contains("· idle") && lines[1].contains("#3 succeeded 5m ago"),
        "{text}"
    );
    assert!(lines[1].contains("holding deployed"), "{text}");
    assert!(text.contains("↳ Full Disk Access missing: grant /Library/Application Support/coeus/coeus-launcher in System Settings > Privacy & Security > Full Disk Access"));
    assert!(text.contains("● building 58%"), "{text}");
    assert!(text.contains("deployed → caf4e2b"), "{text}");
    assert!(text.contains("↳ reboot required"), "{text}");
    assert!(text.contains("↳ boot pending"), "{text}");
    assert!(text.contains("⊘ unreachable"), "{text}");
    assert!(text.contains("↳ connection refused"), "{text}");
}

#[test]
fn history_lists_jobs_newest_first_with_failures_explained() {
    let jobs = vec![
        (host("lab"), failed(12)),
        (host("altar"), summary(3, JobState::Queued)),
    ];
    let lines = plain(&history::jobs(&jobs, NOW));

    assert!(lines[0].starts_with("HOST   JOB  STATE"), "{}", lines[0]);
    assert!(
        lines[1].starts_with(
            "lab    #12  ✗ failed  switch  main@caf4e2b  r3dlust@altar  10m ago  4m00s"
        ),
        "{}",
        lines[1]
    );
    assert!(lines[2].ends_with("in building: builder for 'hello.drv' failed with exit code 1"));
    assert!(lines[3].starts_with("altar  #3   queued"), "{}", lines[3]);
}

#[test]
fn generations_mark_the_running_and_booted_systems() {
    let generation = |number, current, booted| Generation {
        number,
        toplevel: TOPLEVEL.parse().unwrap(),
        created_at: ORDERED,
        current,
        booted,
    };
    let lines = plain(&history::generations(
        &[generation(42, true, false), generation(41, false, true)],
        NOW,
    ));

    assert!(
        lines[1].starts_with("42   current  10m ago"),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].starts_with("41   booted   10m ago"),
        "{}",
        lines[2]
    );
    assert!(lines[1].ends_with("nixos-system-lab-26.11"));
}

#[test]
fn closure_diffs_list_changes_and_totals() {
    let change = |name: &str, before: &[&str], after: &[&str], size_delta| PackageChange {
        name: name.into(),
        before: before.iter().map(|version| version.to_string()).collect(),
        after: after.iter().map(|version| version.to_string()).collect(),
        size_delta,
    };
    let changes = ClosureDiff {
        changes: vec![
            change("firefox", &["120.0"], &["121.0"], 1024 * 1024),
            change("htop", &[], &["3.3.0"], 300 * 1024),
            change("nano", &["8.0"], &[], -100 * 1024),
        ],
    };

    let lines = plain(&diff::lines(&changes, Some(2)));
    assert!(
        lines[1].starts_with("~  firefox  120.0 → 121.0  +1.0 MiB"),
        "{}",
        lines[1]
    );
    assert!(lines[2].starts_with("+  htop     3.3.0"), "{}", lines[2]);
    assert_eq!(lines[3], "… 1 more");
    assert_eq!(lines[4], "1 added · 1 removed · 1 changed · +1.2 MiB");
}
