use super::{Fleet, enroll, git};

#[tokio::test]
async fn a_first_open_cuts_the_branch_from_the_default_branch() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;

    assert_eq!(branch.base(), "main");
    assert_eq!(branch.clone_dir(), fleet.state("altar").clone_dir());
    let clone = branch.clone_dir();
    assert_eq!(
        git(clone, &["branch", "--show-current"]),
        "coeus/bootstrap/nova"
    );
    assert_eq!(
        branch.head().await.unwrap().to_string(),
        git(&fleet.checkout, &["rev-parse", "HEAD"])
    );
    assert_eq!(branch.enrolled().await.unwrap(), None);

    // the person's checkout is never touched
    assert_eq!(git(&fleet.checkout, &["branch", "--show-current"]), "main");
    assert_eq!(git(&fleet.checkout, &["branch", "--list", "coeus/*"]), "");
}

#[tokio::test]
async fn the_clone_does_not_depend_on_the_checkout() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    let alternates = branch.clone_dir().join(".git/objects/info/alternates");

    let borrowed = std::fs::read_to_string(alternates).unwrap_or_default();
    assert_eq!(borrowed.trim(), "");
}

#[tokio::test]
async fn a_half_made_clone_is_cloned_again() {
    let fleet = Fleet::new();
    let state = fleet.state("altar");
    std::fs::create_dir_all(state.clone_dir().join("hosts")).unwrap();
    std::fs::write(state.clone_dir().join("hosts/stray"), "").unwrap();
    std::fs::create_dir_all(state.root().join("repo.partial")).unwrap();

    let branch = fleet.open("altar").await;

    assert!(!branch.clone_dir().join("hosts/stray").exists());
    assert!(!state.root().join("repo.partial").exists());
    assert_eq!(branch.uncommitted().await.unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn a_stale_index_lock_is_removed() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    enroll(&branch).await;
    let head = branch.head().await.unwrap();

    let dot_git = branch.clone_dir().join(".git");
    std::fs::write(dot_git.join("index.lock"), "").unwrap();
    drop(branch);
    let branch = fleet.open("altar").await;

    assert!(!dot_git.join("index.lock").exists());
    assert_eq!(branch.head().await.unwrap(), head);
    std::fs::write(branch.path(&super::path("hosts/nova/extra")), "").unwrap();
    assert_eq!(branch.uncommitted().await.unwrap(), ["hosts/nova/extra"]);
}

#[tokio::test]
async fn a_rebase_stopped_midway_is_aborted() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    enroll(&branch).await;
    let head = branch.head().await.unwrap();
    fleet.commit_on_main(&[("hosts/nova/flake.nix", "{ taken = true; }\n")]);

    let clone = branch.clone_dir().to_owned();
    git(&clone, &["fetch", "--quiet", "origin"]);
    let stopped = std::process::Command::new("git")
        .arg("-C")
        .arg(&clone)
        .args(["rebase", "origin/main"])
        .output()
        .unwrap();
    assert!(!stopped.status.success());
    drop(branch);

    let branch = fleet.open("altar").await;

    assert!(!clone.join(".git/rebase-merge").exists());
    assert_eq!(branch.head().await.unwrap(), head);
}

#[tokio::test]
async fn a_branch_only_on_origin_is_picked_up() {
    let fleet = Fleet::new();
    let first = fleet.open("altar").await;
    let enrolled = enroll(&first).await;
    first.push().await.unwrap();

    let second = fleet.open("vesta").await;

    assert_eq!(second.enrolled().await.unwrap(), Some(enrolled));
    assert_eq!(second.head().await.unwrap(), first.head().await.unwrap());
}

#[tokio::test]
async fn a_local_branch_behind_what_origin_shows_is_moved_to_it() {
    let fleet = Fleet::new();
    let stale = fleet.open("altar").await;
    let other = fleet.open("vesta").await;
    enroll(&other).await;
    other.push().await.unwrap();
    drop(stale);

    let reopened = fleet.open("altar").await;

    assert_eq!(reopened.head().await.unwrap(), other.head().await.unwrap());
}
