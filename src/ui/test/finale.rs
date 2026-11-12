use std::path::Path;

use eyre::eyre;

use super::*;
use crate::{
    bootstrap::{
        Blocker, Evidence, Finale, Step,
        branch::{PullRequest, PullState, body},
        fleet::Reboot,
        state::StateDir,
    },
    common::model::{Failure, FdaStatus, JobState, Outcome, Phase},
    ui::finale,
};

fn state() -> StateDir {
    StateDir::under(Path::new("/state"), &host("nova"))
}

fn evidence(state: PullState) -> Evidence {
    Evidence {
        host: host("nova"),
        pull_request: PullRequest {
            number: 142,
            url: "https://github.com/acme/nix/pull/142".into(),
            draft: false,
            state,
            body: String::new(),
        },
        section: body::Evidence::default(),
        notes: Vec::new(),
        reboot: Reboot::NotNeeded,
        full_disk_access: FdaStatus::NotApplicable,
    }
}

fn said(finale: &Finale) -> Vec<String> {
    plain(&finale::lines(&state(), finale))
}

#[test]
fn done_says_to_merge_the_pull_request() {
    let lines = said(&Finale::Done(Box::new(evidence(PullState::Open))));
    assert_eq!(
        lines,
        [
            "[✓] nova is in the fleet, waiting for review",
            "    merge https://github.com/acme/nix/pull/142: every other machine learns about nova on the next rollout",
        ]
    );
}

#[test]
fn done_and_merged_has_nothing_left() {
    let lines = said(&Finale::Done(Box::new(evidence(PullState::Merged))));
    assert_eq!(lines, ["[✓] nova is in the fleet"]);
}

#[test]
fn next_steps_follow_the_evidence() {
    let mut evidence = evidence(PullState::Open);
    evidence.reboot = Reboot::Declined;
    evidence.full_disk_access = FdaStatus::Missing;
    evidence.notes = vec!["add nova to the backup rotation".into()];
    let lines = said(&Finale::Done(Box::new(evidence)));

    assert!(
        lines[2].contains("`coeus reboot nova` when it suits"),
        "{lines:?}"
    );
    assert!(
        lines[3].contains("on nova itself, run `coeus doctor --fix`"),
        "{lines:?}"
    );
    assert_eq!(lines[4], "    add nova to the backup rotation");
}

#[test]
fn a_detach_during_the_switch_says_it_goes_on() {
    let lines = said(&Finale::Detached { at: Step::Switch });
    assert_eq!(
        lines[0],
        "[•] detached during the switch: nova goes on switching alone"
    );
    assert_eq!(
        lines[1],
        "    running `coeus bootstrap nova` again picks up from there"
    );

    let lines = said(&Finale::Detached { at: Step::Rekey });
    assert_eq!(lines[0], "[•] stopped during rekey; nothing runs on nova");
}

#[test]
fn a_failed_switch_shows_its_tail_and_both_ways_out() {
    let mut failure = Failure::new(Phase::Verifying, "tailscale is not running");
    failure.tail = (1..=12).map(|n| format!("line {n}")).collect();
    let mut job = summary(1, JobState::Finished(Outcome::RolledBack(failure)));
    job.attempt = 1;
    let lines = said(&Finale::Blocked {
        at: Step::Switch,
        blocker: Blocker::SwitchFailed { job: Box::new(job) },
    });

    assert_eq!(
        lines[0],
        "[✗] nova's first switch, job #1, rolled back in verifying: tailscale is not running"
    );
    let tail: Vec<&String> = lines.iter().filter(|line| line.contains("│")).collect();
    assert_eq!(tail.len(), 8);
    assert_eq!(tail[7], "    │ line 12");
    assert!(
        lines
            .iter()
            .any(|line| line == "    nova runs the system it ran before")
    );
    assert!(
        lines.iter().any(|line| line.contains("commit a fix on coeus/bootstrap/nova in /state/coeus/bootstrap/nova/repo, or `coeus bootstrap nova --retry`")),
        "{lines:#?}"
    );
    assert_eq!(
        lines.last().unwrap(),
        "    then `coeus bootstrap nova` picks up from there"
    );
}

#[test]
fn blocks_say_what_a_person_must_do() {
    let files = (0..7)
        .map(|n| format!("secrets/s{n}.age").parse().unwrap())
        .collect();
    let lines = said(&Finale::Blocked {
        at: Step::Rekey,
        blocker: Blocker::NotAdmin { files },
    });
    assert_eq!(
        lines[0],
        "[⚠] this machine cannot open 7 secrets, so it is not an admin of the fleet"
    );
    assert_eq!(lines[1], "    secrets/s0.age");
    assert_eq!(lines[6], "    … and 2 more");

    let lines = said(&Finale::Blocked {
        at: Step::Rekey,
        blocker: Blocker::Unreadable {
            files: vec![(
                "secrets/yubi.age".parse().unwrap(),
                "it is also encrypted to a piv-p256 recipient, which coeus cannot keep".into(),
            )],
        },
    });
    assert_eq!(
        lines[0],
        "[⚠] coeus cannot rekey 1 secret without losing a recipient it does not know"
    );
    assert!(lines[1].contains("secrets/yubi.age: it is also encrypted to a piv-p256"));
    assert!(lines[2].contains("leave them out"), "{lines:?}");

    let lines = said(&Finale::Blocked {
        at: Step::Enroll,
        blocker: Blocker::Unanswered { id: "type".into() },
    });
    assert!(lines[1].contains("`--answer type=<value>`"), "{lines:?}");
}

#[test]
fn failures_name_the_step_and_the_error() {
    let lines = said(&Finale::Unreached {
        at: Step::Reach,
        error: eyre!("connection refused"),
    });
    assert_eq!(lines[0], "[✗] could not reach nova: connection refused");

    let lines = said(&Finale::Failed {
        at: Step::Push,
        error: eyre!("rejected").wrap_err("git push"),
    });
    assert_eq!(lines[0], "[✗] push failed: git push: rejected");
}
