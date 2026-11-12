use std::time::Duration;

use crate::{
    common::model::{Pinned, Source, Timestamp},
    config::FollowSettings,
    engine::{
        follow::{Deployed, Follower},
        lookout::Schedule,
    },
    nix::FlakeRepo,
};

const OLD: &str = "1111111111111111111111111111111111111111";
const NEW: &str = "2222222222222222222222222222222222222222";

const INTERVAL: Duration = Duration::from_secs(900);

fn follower(latest: &str, committed_at: i64) -> Follower {
    Follower::seeing(
        "deployed",
        Some(Pinned {
            git_ref: "deployed".into(),
            rev: latest.parse().unwrap(),
            committed_at: Timestamp(committed_at),
        }),
    )
}

fn git(rev: &str) -> Source {
    Source::Git {
        rev: rev.parse().unwrap(),
        git_ref: Some("main".into()),
    }
}

#[test]
fn catches_up_when_the_ref_moved_past_the_last_deployment() {
    let source = git(OLD);
    let deployed = Deployed {
        source: Some(&source),
        at: Timestamp(1_000),
    };

    let behind = follower(NEW, 2_000).behind(Some(&deployed));
    let expected = Source::Git {
        rev: NEW.parse().unwrap(),
        git_ref: Some("deployed".into()),
    };
    assert_eq!(behind, Some(expected));
}

#[test]
fn stays_when_the_machine_runs_the_ref() {
    let source = git(NEW);
    let deployed = Deployed {
        source: Some(&source),
        at: Timestamp(1_000),
    };

    let follower = follower(NEW, 2_000);
    assert_eq!(follower.behind(Some(&deployed)), None);
    assert!(!follower.state(Some(&deployed)).paused);
}

#[test]
fn a_later_hand_deploy_holds_until_the_ref_moves_again() {
    let source = Source::Generation(9);
    let deployed = Deployed {
        source: Some(&source),
        at: Timestamp(3_000),
    };

    let held = follower(NEW, 2_000);
    assert_eq!(held.behind(Some(&deployed)), None);
    assert!(held.state(Some(&deployed)).paused);

    let moved = follower(NEW, 4_000);
    assert!(moved.behind(Some(&deployed)).is_some());
}

#[test]
fn a_machine_coeus_never_deployed_catches_up() {
    assert!(follower(NEW, 2_000).behind(None).is_some());
}

#[test]
fn a_follower_that_has_not_looked_yet_does_nothing() {
    let follower = Follower::seeing("deployed", None);
    assert_eq!(follower.behind(None), None);
    assert!(!follower.state(None).paused);
}

#[test]
fn waits_a_moment_after_starting_before_the_first_look() {
    let start = Timestamp(1_000_000);
    let schedule = Schedule::new(INTERVAL, start);

    assert!(!schedule.due(start), "the first look waits for coeusd to settle");
    assert!(schedule.due(start.after(Schedule::SETTLE)));
}

#[test]
fn looks_again_sooner_after_a_failure_and_backs_off_up_to_the_interval() {
    let now = Timestamp(1_000_000);
    let mut schedule = Schedule::new(INTERVAL, now);

    let waits: Vec<u64> = (0..7).map(|_| schedule.failed(now).as_secs()).collect();
    assert_eq!(waits, [30, 60, 120, 240, 480, 900, 900]);
    assert!(!schedule.due(now));
    assert!(schedule.due(now.after(INTERVAL)));

    schedule.succeeded(now);
    assert_eq!(schedule.until(now), INTERVAL);
    assert_eq!(schedule.failed(now), Schedule::RETRY, "a success starts over");
}

#[test]
fn a_machine_that_slept_past_its_look_looks_on_waking() {
    let now = Timestamp(1_000_000);
    let mut schedule = Schedule::new(INTERVAL, now);
    schedule.succeeded(now);

    let woke = now.after(Duration::from_secs(8 * 3600));
    assert!(schedule.due(woke));
    assert_eq!(schedule.until(woke), Duration::ZERO);
}

#[tokio::test]
async fn refuses_to_follow_what_is_not_a_branch_or_tag() {
    let settings = |git_ref: &str| FollowSettings {
        git_ref: git_ref.into(),
        interval: INTERVAL,
    };
    let repo = || FlakeRepo::new("git+https://example.com/nix");

    assert!(Follower::spawn(repo(), settings("--upload-pack=x")).is_err());
    assert!(Follower::spawn(repo(), settings("deployed")).is_ok());
}

#[test]
fn a_machine_switched_outside_coeus_holds_until_the_ref_moves() {
    let deployed = Deployed {
        source: None,
        at: Timestamp(3_000),
    };

    let held = follower(NEW, 2_000);
    assert_eq!(held.behind(Some(&deployed)), None);
    assert!(held.state(Some(&deployed)).paused);

    assert!(follower(NEW, 4_000).behind(Some(&deployed)).is_some());
}
