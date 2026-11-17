use std::{
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    common::model::{Pinned, Timestamp},
    config::FollowSettings,
    engine::lookout::{Lookout, Schedule, publish},
    nix::FlakeRepo,
    testing::{GitFlake, Harness},
};

fn follow(git_ref: &str, state_file: Option<PathBuf>) -> FollowSettings {
    FollowSettings {
        git_ref: git_ref.into(),
        interval: Duration::from_secs(15 * 60),
        state_file,
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// what the state file should say about the repository's head, checked
/// between `before` and now
fn assert_published(path: &Path, repo: &GitFlake, before: Timestamp) {
    let published = read(path);
    let checked = published["checked"].as_i64().unwrap();
    assert!(
        (before.as_secs()..=Timestamp::now().as_secs()).contains(&checked),
        "{published}"
    );

    let committed: i64 = repo.git(&["log", "-1", "--format=%ct"]).parse().unwrap();
    let expected = json!({
        "ref": "deployed",
        "rev": repo.git(&["rev-parse", "HEAD"]),
        "committed": committed,
        "checked": checked,
    });
    assert_eq!(published, expected);
}

fn pinned(rev: &str, committed_at: i64) -> Pinned {
    Pinned {
        git_ref: "deployed".into(),
        rev: rev.repeat(40).parse().unwrap(),
        committed_at: Timestamp(committed_at),
    }
}

#[tokio::test]
async fn a_scheduled_look_publishes_where_the_ref_points() {
    let repo = GitFlake::init();
    if !repo.resolvable() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run/coeus/follow.json");

    let before = Timestamp::now();
    let settled = Timestamp(before.0 - Schedule::SETTLE.as_millis() as i64);
    let (mut latest, _asks) = Lookout::spawn(
        FlakeRepo::new(repo.url()),
        follow("deployed", Some(path.clone())),
        settled,
    );
    tokio::time::timeout(Duration::from_secs(60), latest.changed())
        .await
        .unwrap()
        .unwrap();

    assert_published(&path, &repo, before);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o644,
        "programs running as other users read it"
    );
}

#[tokio::test]
async fn catching_up_publishes_where_the_ref_points() {
    let repo = GitFlake::init();
    if !repo.resolvable() {
        return;
    }
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("follow.json");
    let harness = Harness::following(
        tempfile::tempdir().unwrap(),
        repo.url(),
        follow("deployed", Some(path.clone())),
    )
    .await;

    let before = Timestamp::now();
    harness.engine.catch_up().await.unwrap().unwrap();
    assert_published(&path, &repo, before);

    repo.git(&["commit", "--quiet", "--allow-empty", "-m", "second"]);
    harness.engine.catch_up().await.unwrap().unwrap();
    assert_published(&path, &repo, before);
    harness.shutdown.cancel();
}

#[tokio::test]
async fn a_failed_lookup_leaves_the_file_as_it_was() {
    let repo = GitFlake::init();
    let state = tempfile::tempdir().unwrap();
    let path = state.path().join("follow.json");
    publish(&path, &pinned("1", 1_000_000), Timestamp(2_000_000));
    let before = std::fs::read(&path).unwrap();

    let harness = Harness::following(
        tempfile::tempdir().unwrap(),
        repo.url(),
        follow("missing", Some(path.clone())),
    )
    .await;
    assert!(harness.engine.catch_up().await.unwrap().is_err());

    assert_eq!(std::fs::read(&path).unwrap(), before);
    harness.shutdown.cancel();
}

#[test]
fn the_file_is_replaced_whole_and_nothing_is_left_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("follow.json");

    publish(&path, &pinned("1", 1_000_999), Timestamp(2_000_000));
    let mut reader = std::fs::File::open(&path).unwrap();
    publish(&path, &pinned("2", 3_000_000), Timestamp(4_000_000));

    let mut old = String::new();
    reader.read_to_string(&mut old).unwrap();
    let old: Value = serde_json::from_str(&old).unwrap();
    assert_eq!(old["committed"], 1_000, "a reader keeps the file it opened");
    assert_eq!(old["checked"], 2_000);
    assert_eq!(read(&path)["rev"], "2".repeat(40));

    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, ["follow.json"]);
}

#[tokio::test]
async fn a_file_it_cannot_write_leaves_the_follower_working() {
    let repo = GitFlake::init();
    if !repo.resolvable() {
        return;
    }
    let state = tempfile::tempdir().unwrap();
    let blocker = state.path().join("not-a-directory");
    std::fs::write(&blocker, "").unwrap();
    let harness = Harness::following(
        tempfile::tempdir().unwrap(),
        repo.url(),
        follow("deployed", Some(blocker.join("follow.json"))),
    )
    .await;

    let found = harness.engine.catch_up().await.unwrap().unwrap();
    let head = repo.git(&["rev-parse", "HEAD"]);
    assert_eq!(found.rev.to_string(), head);
    harness
        .until(|view| {
            view.system
                .follow
                .as_ref()
                .and_then(|follow| follow.latest.as_ref())
                .is_some_and(|latest| latest.rev.to_string() == head)
        })
        .await;
    harness.shutdown.cancel();
}
