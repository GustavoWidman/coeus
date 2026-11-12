use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::CoeusServer;
use crate::{
    client::{AttachmentEvent, CoeusClient},
    common::model::{
        Action, AttachFrame, JobEvent, JobId, JobSelector, JobState, NixOptions, Order, OrderId,
        Origin, Outcome, Placement, Source,
    },
    config::{ClientSettings, Endpoint, Key},
    testing::Harness,
};

struct Served {
    harness: Harness,
    endpoint: Endpoint,
    key: Key,
    restart: CancellationToken,
}

impl Served {
    async fn start() -> Self {
        let harness = Harness::start(tempfile::tempdir().unwrap()).await;
        let key = Key::generate();
        let server =
            CoeusServer::bind("127.0.0.1:0".parse().unwrap(), &key, harness.engine.clone())
                .await
                .unwrap();
        let endpoint = Endpoint {
            host: "127.0.0.1".into(),
            port: server.local_addr().unwrap().port(),
        };

        let restart = CancellationToken::new();
        tokio::spawn(server.run(restart.clone()));

        Self {
            harness,
            endpoint,
            key,
            restart,
        }
    }

    async fn client(&self) -> CoeusClient {
        CoeusClient::connect(&ClientSettings::default(), &self.endpoint, &self.key)
            .await
            .unwrap()
    }

    fn stop(&self) {
        self.restart.cancel();
        self.harness.shutdown.cancel();
    }
}

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
        .attach(JobSelector::Current, 0)
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
        .attach(JobSelector::Id(JobId(9)), 0)
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
