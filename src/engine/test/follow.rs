use std::time::Duration;

use crate::{
    common::model::{Pinned, Source, Timestamp},
    config::FollowSettings,
    engine::follow::{Deployed, Follower},
    nix::FlakeRepo,
};

const OLD: &str = "1111111111111111111111111111111111111111";
const NEW: &str = "2222222222222222222222222222222222222222";

fn follower(latest: &str, committed_at: i64) -> Follower {
    let mut follower = Follower::new(
        FlakeRepo::new("git+https://example.com/nix"),
        FollowSettings {
            git_ref: "deployed".into(),
            interval: Duration::from_secs(900),
        },
    );
    follower.saw(Pinned {
        git_ref: "deployed".into(),
        rev: latest.parse().unwrap(),
        committed_at: Timestamp(committed_at),
    });
    follower
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
        source: &source,
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
        source: &source,
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
        source: &source,
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
fn waits_a_moment_after_starting_before_the_first_check() {
    let follower = Follower::new(
        FlakeRepo::new("git+https://example.com/nix"),
        FollowSettings {
            git_ref: "deployed".into(),
            interval: Duration::from_secs(900),
        },
    );
    assert!(
        !follower.due(),
        "the first check waits for coeusd to settle"
    );
}
