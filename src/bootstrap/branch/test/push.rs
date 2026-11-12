use super::{Fleet, enroll, path};
use crate::bootstrap::branch::Rebased;

#[tokio::test]
async fn a_push_puts_the_head_on_origin_once() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    enroll(&branch).await;
    assert_eq!(fleet.on_origin(), None);

    branch.push().await.unwrap();
    let head = branch.head().await.unwrap().to_string();
    assert_eq!(fleet.on_origin().as_deref(), Some(head.as_str()));

    branch.push().await.unwrap();
    assert_eq!(fleet.on_origin(), Some(head));
}

#[tokio::test]
async fn a_rebased_branch_replaces_what_this_run_saw() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    enroll(&branch).await;
    branch.push().await.unwrap();
    fleet.commit_on_main(&[("readme.md", "moved\n")]);
    branch.behind().await.unwrap();
    assert_eq!(
        branch.rebase_dropping_rekey().await.unwrap(),
        Rebased::Clean
    );

    branch.push().await.unwrap();

    let head = branch.head().await.unwrap().to_string();
    assert_eq!(fleet.on_origin(), Some(head));
}

#[tokio::test]
async fn the_lease_refuses_a_branch_another_run_pushed_meanwhile() {
    let fleet = Fleet::new();
    let ours = fleet.open("altar").await;
    let theirs = fleet.open("vesta").await;
    enroll(&theirs).await;
    theirs.push().await.unwrap();
    let pushed = theirs.head().await.unwrap().to_string();

    enroll(&ours).await;
    let error = ours.push().await.unwrap_err();

    assert!(
        format!("{error:#}").contains("moved since this run read it"),
        "{error:#}"
    );
    assert_eq!(fleet.on_origin(), Some(pushed));
}

#[tokio::test]
async fn the_lease_refuses_a_branch_moved_after_it_was_read() {
    let fleet = Fleet::new();
    let first = fleet.open("altar").await;
    enroll(&first).await;
    first.push().await.unwrap();
    let ours = fleet.open("vesta").await;

    std::fs::write(first.path(&path("secrets/cache.age")), "resealed\n").unwrap();
    first
        .commit_rekey("chore(nova): rekey", &[path("secrets/cache.age")])
        .await
        .unwrap();
    first.push().await.unwrap();

    fleet.commit_on_main(&[("readme.md", "moved\n")]);
    ours.behind().await.unwrap();
    ours.rebase_dropping_rekey().await.unwrap();
    let error = ours.push().await.unwrap_err();
    assert!(
        format!("{error:#}").contains("moved since this run read it"),
        "{error:#}"
    );
    assert_eq!(
        fleet.on_origin(),
        Some(first.head().await.unwrap().to_string())
    );
}
