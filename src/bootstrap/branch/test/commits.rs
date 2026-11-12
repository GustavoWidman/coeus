use super::{Fleet, enroll, files, git, path, subjects};
use crate::bootstrap::branch::Rebased;

#[tokio::test]
async fn write_adds_only_absent_files_and_stages_every_declared_one() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    std::fs::create_dir_all(branch.path(&path("hosts/nova"))).unwrap();
    std::fs::write(
        branch.path(&path("hosts/nova/flake.nix")),
        "{ mine = 1; }\n",
    )
    .unwrap();

    let declared = files(&[
        ("hosts/nova/flake.nix", "{ }\n"),
        ("hosts/nova/hardware.nix", "{ hw = 1; }\n"),
    ]);
    let written = branch.write(&declared).await.unwrap();

    assert_eq!(written, [path("hosts/nova/hardware.nix")]);
    let kept = std::fs::read_to_string(branch.path(&path("hosts/nova/flake.nix"))).unwrap();
    assert_eq!(kept, "{ mine = 1; }\n");
    let staged = git(branch.clone_dir(), &["diff", "--cached", "--name-only"]);
    assert_eq!(staged, "hosts/nova/flake.nix\nhosts/nova/hardware.nix");
    assert_eq!(branch.write(&declared).await.unwrap(), []);
}

#[tokio::test]
async fn the_enroll_commit_is_found_by_its_trailer() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    std::fs::write(branch.clone_dir().join("unrelated"), "").unwrap();

    let enrolled = enroll(&branch).await;

    assert_eq!(enrolled.subject, "feat(nova): join the fleet");
    assert_eq!(enrolled.commit, branch.head().await.unwrap());
    assert_eq!(
        enrolled.files,
        [
            path("hosts/nova/flake.nix"),
            path("hosts/nova/host.pub"),
            path("hosts/nova/password.age"),
        ]
    );
    let message = git(branch.clone_dir(), &["log", "-1", "--format=%B"]);
    assert_eq!(
        message,
        "feat(nova): join the fleet\n\nCoeus-Bootstrap: enroll nova"
    );
    assert_eq!(branch.enrolled().await.unwrap(), Some(enrolled));
    // the person's other changes stay out of it
    assert_eq!(branch.uncommitted().await.unwrap(), ["unrelated"]);
}

#[tokio::test]
async fn paths_already_on_the_default_branch_are_reported() {
    let fleet = Fleet::new();
    fleet.commit_on_main(&[("hosts/nova/flake.nix", "{ }\n")]);
    let branch = fleet.open("altar").await;

    let flake = path("hosts/nova/flake.nix");
    let hardware = path("hosts/nova/hardware.nix");
    let found = branch.on_base(&[&flake, &hardware]).await.unwrap();

    assert_eq!(found, [flake]);
}

#[tokio::test]
async fn secrets_are_the_tracked_age_files() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    enroll(&branch).await;
    std::fs::write(branch.clone_dir().join("untracked.age"), "").unwrap();

    let secrets = branch.secrets().await.unwrap();

    assert_eq!(
        secrets,
        [path("hosts/nova/password.age"), path("secrets/cache.age")]
    );
}

#[tokio::test]
async fn a_second_rekey_replaces_the_first_on_top() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    enroll(&branch).await;
    let cache = path("secrets/cache.age");
    let other = path("secrets/other.age");

    std::fs::write(branch.path(&cache), "resealed\n").unwrap();
    branch
        .commit_rekey("chore(nova): rekey", std::slice::from_ref(&cache))
        .await
        .unwrap();
    std::fs::write(branch.path(&other), "sealed for nova\n").unwrap();
    let head = branch
        .commit_rekey("chore(nova): rekey", &[other])
        .await
        .unwrap();

    assert_eq!(head, branch.head().await.unwrap());
    assert_eq!(
        subjects(&branch),
        ["feat(nova): join the fleet", "chore(nova): rekey"]
    );
    let top = git(
        branch.clone_dir(),
        &["show", "--name-only", "--format=%(trailers)", "HEAD"],
    );
    assert_eq!(
        top,
        "Coeus-Bootstrap: rekey nova\n\n\nsecrets/cache.age\nsecrets/other.age"
    );
}

#[tokio::test]
async fn a_moved_default_branch_is_rebased_onto_without_the_rekey() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    enroll(&branch).await;
    std::fs::write(branch.path(&path("secrets/cache.age")), "resealed\n").unwrap();
    let rekeyed = path("secrets/cache.age");
    branch
        .commit_rekey("chore(nova): rekey", &[rekeyed])
        .await
        .unwrap();
    assert_eq!(branch.behind().await.unwrap(), 0);

    fleet.commit_on_main(&[("secrets/new.age", "sealed\n")]);
    fleet.commit_on_main(&[("readme.md", "the fleet, grown\n")]);
    assert_eq!(branch.behind().await.unwrap(), 2);

    let rebased = branch.rebase_dropping_rekey().await.unwrap();

    assert_eq!(rebased, Rebased::Clean);
    assert_eq!(branch.behind().await.unwrap(), 0);
    assert_eq!(subjects(&branch), ["feat(nova): join the fleet"]);
    let enrolled = branch.enrolled().await.unwrap().unwrap();
    assert_eq!(enrolled.commit, branch.head().await.unwrap());
    assert_eq!(enrolled.files.len(), 3);
    let main = git(branch.clone_dir(), &["rev-parse", "origin/main"]);
    assert_eq!(git(branch.clone_dir(), &["rev-parse", "HEAD~1"]), main);
    let cache = std::fs::read_to_string(branch.path(&path("secrets/cache.age"))).unwrap();
    assert_eq!(cache, "sealed\n");
}

#[tokio::test]
async fn a_conflicting_rebase_is_aborted_and_the_branch_kept() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    enroll(&branch).await;
    let head = branch.head().await.unwrap();
    fleet.commit_on_main(&[("hosts/nova/flake.nix", "{ taken = true; }\n")]);
    assert_eq!(branch.behind().await.unwrap(), 1);

    let rebased = branch.rebase_dropping_rekey().await.unwrap();

    assert_eq!(rebased, Rebased::Conflict);
    assert_eq!(branch.head().await.unwrap(), head);
    assert!(!branch.clone_dir().join(".git/rebase-merge").exists());
    assert_eq!(branch.uncommitted().await.unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn a_branch_of_only_a_rekey_rebases_to_the_default_branch() {
    let fleet = Fleet::new();
    let branch = fleet.open("altar").await;
    std::fs::write(branch.path(&path("secrets/cache.age")), "resealed\n").unwrap();
    branch
        .commit_rekey("chore(nova): rekey", &[path("secrets/cache.age")])
        .await
        .unwrap();
    fleet.commit_on_main(&[("readme.md", "moved\n")]);
    branch.behind().await.unwrap();

    assert_eq!(
        branch.rebase_dropping_rekey().await.unwrap(),
        Rebased::Clean
    );
    assert_eq!(subjects(&branch), Vec::<String>::new());
}
