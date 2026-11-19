use std::time::Duration;

use crate::{
    client::AttachmentEvent,
    common::{
        model::{
            Action, AttachFrame, JobEvent, JobId, JobSelector, JobState, NixOptions, Order,
            OrderId, Origin, Outcome, Placement, Source,
        },
        proto::TerminalSize,
    },
    config::FollowSettings,
    testing::{GitFlake, Harness, Served},
};

fn order(id: u64) -> Order {
    Order {
        id: OrderId(id),
        source: Source::Git {
            rev: "caf4e2b9a1d6c3e8f0b2a4c6d8e0f1a3b5c7d9e1".parse().unwrap(),
            git_ref: Some("main".into()),
        },
        action: Action::Build,
        options: NixOptions::default(),
        origin: Origin::Follow,
        now: false,
    }
}

#[tokio::test]
async fn orders_and_attaches_until_the_job_ends() {
    let served = Served::start().await;

    let admission = served
        .client()
        .await
        .deploy(order(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admission.placement, Placement::Started);

    let mut attachment = served
        .client()
        .await
        .attach(JobSelector::Current, 0, None)
        .await
        .unwrap();
    let Some(AttachmentEvent::Started(job)) = attachment.next().await.unwrap() else {
        panic!("an attach starts by naming its job");
    };
    assert_eq!(job.id, JobId(1));

    served.harness.release.notify_one();

    let mut events = Vec::new();
    let outcome = loop {
        match attachment.next().await.unwrap() {
            Some(AttachmentEvent::Frames { frames, .. }) => {
                events.extend(frames.into_iter().filter_map(|frame| match frame {
                    AttachFrame::Event(event) => Some(event.event),
                    AttachFrame::Nix(_) => None,
                }));
            }
            Some(AttachmentEvent::Ended { outcome, .. }) => break outcome,
            other => panic!("unexpected {other:?}"),
        }
    };

    assert_eq!(outcome, Outcome::Succeeded);
    assert!(matches!(events.first(), Some(JobEvent::Started { .. })));
    assert!(matches!(
        events.last(),
        Some(JobEvent::Finished(Outcome::Succeeded))
    ));
    served.stop();
}

#[tokio::test]
async fn watches_follow_the_machine_and_history_keeps_finished_jobs() {
    let served = Served::start().await;
    let mut views = served.client().await.watch().await.unwrap();
    assert!(views.next().await.unwrap().unwrap().running.is_none());

    served
        .client()
        .await
        .deploy(order(1))
        .await
        .unwrap()
        .unwrap();
    let running = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = views.next().await.unwrap().unwrap();
            if let Some(running) = view.running {
                return running;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(running.summary.id, JobId(1));

    served.harness.release.notify_one();
    served
        .harness
        .until(|view| view.recent.iter().any(|job| job.id == JobId(1)))
        .await;

    let history = served.client().await.history(10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobState::Finished(Outcome::Succeeded));
    served.stop();
}

#[tokio::test]
async fn errors_travel_as_messages() {
    let served = Served::start().await;
    let error = served
        .client()
        .await
        .attach(JobSelector::Id(JobId(9)), 0, None)
        .await
        .unwrap()
        .next()
        .await;
    assert!(error.unwrap_err().to_string().contains("no such job"));
    served.stop();
}

#[tokio::test]
async fn takes_a_working_tree_once() {
    let served = Served::start().await;
    let tree = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(tree.path())
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    };

    git(&["init", "--quiet"]);
    std::fs::write(tree.path().join("flake.nix"), "{ outputs = _: { }; }").unwrap();
    std::fs::write(tree.path().join(".gitignore"), "result\n").unwrap();
    git(&["add", "flake.nix", ".gitignore"]);
    std::fs::write(tree.path().join("new.nix"), "{ }").unwrap();
    std::fs::write(tree.path().join("result"), "ignored").unwrap();

    let snapshot = crate::client::Snapshot::capture(tree.path()).await.unwrap();
    assert!(snapshot.label.contains("dirty)"), "{}", snapshot.label);

    served.client().await.upload(&snapshot).await.unwrap();
    let snapshots = served.harness.engine.snapshots();
    let unpacked = snapshots.path(&snapshot.nar_hash);
    assert!(unpacked.join("flake.nix").is_file());
    assert!(
        unpacked.join("new.nix").is_file(),
        "untracked files are part of the tree"
    );
    assert!(!unpacked.join("result").exists(), "ignored files are not");
    assert!(!unpacked.join(".git").exists());

    served.client().await.upload(&snapshot).await.unwrap();
    served.stop();
}

#[tokio::test]
async fn a_machine_that_follows_nothing_cannot_catch_up() {
    let served = Served::start().await;
    let error = served.client().await.catch_up().await.unwrap_err();
    assert!(error.to_string().contains("follows no ref"), "{error}");
    served.stop();
}

#[tokio::test]
async fn catching_up_looks_the_followed_ref_up_at_once() {
    let repo = GitFlake::init();
    if !repo.resolvable() {
        return;
    }

    let follow = FollowSettings {
        git_ref: "deployed".into(),
        interval: Duration::from_secs(15 * 60),
        state_file: None,
    };
    let harness = Harness::following(tempfile::tempdir().unwrap(), repo.url(), follow).await;
    let served = Served::serve(harness).await;

    let first = served.client().await.catch_up().await.unwrap();
    assert_eq!(first.rev.to_string(), repo.git(&["rev-parse", "HEAD"]));

    repo.git(&["commit", "--quiet", "--allow-empty", "-m", "second"]);
    let second = served.client().await.catch_up().await.unwrap();
    let head = repo.git(&["rev-parse", "HEAD"]);
    assert_eq!(second.rev.to_string(), head, "a second ask looks again");

    served
        .harness
        .until(|view| {
            view.system
                .follow
                .as_ref()
                .and_then(|follow| follow.latest.as_ref())
                .is_some_and(|latest| latest.rev.to_string() == head)
        })
        .await;
    served.stop();
}

#[tokio::test]
async fn a_read_only_key_watches_and_reads_logs() {
    let served = Served::start().await;
    served
        .client()
        .await
        .deploy(order(1))
        .await
        .unwrap()
        .unwrap();
    served.harness.release.notify_one();
    served
        .harness
        .until(|view| view.recent.iter().any(|job| job.id == JobId(1)))
        .await;

    let mut reader = served.reader().await;
    assert_eq!(reader.view().await.unwrap().recent[0].id, JobId(1));
    assert_eq!(reader.history(10).await.unwrap()[0].id, JobId(1));

    let mut views = served.reader().await.watch().await.unwrap();
    assert!(views.next().await.unwrap().is_some());

    let mut attachment = served
        .reader()
        .await
        .attach(JobSelector::Id(JobId(1)), 0, None)
        .await
        .unwrap();
    let outcome = loop {
        match attachment.next().await.unwrap() {
            Some(AttachmentEvent::Ended { outcome, .. }) => break outcome,
            Some(_) => {}
            None => panic!("the attach ended without an outcome"),
        }
    };
    assert_eq!(outcome, Outcome::Succeeded);
    served.stop();
}

#[tokio::test]
async fn a_read_only_key_is_refused_work_and_keeps_its_connection() {
    let served = Served::start().await;
    let mut reader = served.reader().await;
    let refused = |error: eyre::Report| {
        assert!(
            error.to_string().contains("this key may only read"),
            "{error}"
        );
    };

    refused(reader.deploy(order(1)).await.unwrap_err());
    refused(reader.cancel(JobSelector::Current).await.unwrap_err());
    refused(reader.resolve("main").await.unwrap_err());
    refused(reader.catch_up().await.unwrap_err());
    refused(reader.reboot().await.unwrap_err());
    let tree = tempfile::tempdir().unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .arg(tree.path())
        .status()
        .unwrap();
    assert!(init.success());
    std::fs::write(tree.path().join("flake.nix"), "{ }").unwrap();
    let snapshot = crate::client::Snapshot::capture(tree.path()).await.unwrap();
    refused(reader.upload(&snapshot).await.unwrap_err());

    let view = reader.view().await.unwrap();
    assert!(view.running.is_none() && view.recent.is_empty(), "{view:?}");
    assert!(!served.harness.engine.snapshots().has(&snapshot.nar_hash));

    let terminal = TerminalSize {
        columns: 80,
        rows: 24,
    };
    let drawn = reader
        .attach(JobSelector::Current, 0, Some(terminal))
        .await
        .unwrap()
        .next()
        .await;
    refused(drawn.unwrap_err());
    served.stop();
}
