//! the site over a real coeusd, reached with the read-only key

use axum::http::StatusCode;
use tokio_util::sync::CancellationToken;

use super::sse::{Events, body, get, viewer};
use crate::{
    client::{CoeusClient, Fleet, Tailnet, Targets},
    common::{
        model::{Action, NixOptions, Order, OrderId, Origin, Source as Commit},
        proto::Role,
    },
    config::{ClientSettings, Endpoint, HostEntry, Key},
    server::CoeusServer,
    testing::Harness,
    web::{
        site::{Access, Settings, Site},
        source::Source,
    },
};

struct Live {
    harness: Harness,
    site: Site,
    endpoint: Endpoint,
    key: Key,
    stop: CancellationToken,
}

impl Live {
    async fn start() -> Self {
        let harness = Harness::start(tempfile::tempdir().unwrap()).await;
        let (key, read_key) = (Key::generate(), Key::generate());
        let keys = [(Role::Full, key.clone()), (Role::Read, read_key.clone())];
        let server = CoeusServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            &keys,
            harness.engine.clone(),
        )
        .await
        .unwrap();
        let port = server.local_addr().unwrap().port();
        let stop = CancellationToken::new();
        tokio::spawn(server.run(stop.clone()));

        let mut settings = ClientSettings::default();
        settings.hosts.insert(
            "lab".parse().unwrap(),
            HostEntry {
                address: Some("127.0.0.1".into()),
                port: Some(port),
                tags: Vec::new(),
            },
        );
        let targets = Targets::select(&["@all".to_owned()], &settings).unwrap();
        let fleet = Fleet::new(settings, read_key, targets).seeing(Tailnet::default());
        let source = Source::Live(fleet.clone());
        let site = Site::start(
            fleet.hosts().to_vec(),
            source.clone(),
            Settings {
                web_dir: None,
                served_by: "lab".parse().unwrap(),
                access: Access::Open(viewer()),
            },
        );
        source.feed(site.inputs.clone());

        Self {
            harness,
            site,
            endpoint: Endpoint {
                host: "127.0.0.1".into(),
                port,
            },
            key,
            stop,
        }
    }

    /// orders a build on lab, which runs until the harness releases it
    async fn order(&self) {
        let order = Order {
            id: OrderId(1),
            source: Commit::Git {
                rev: "caf4e2b9a1d6c3e8f0b2a4c6d8e0f1a3b5c7d9e1".parse().unwrap(),
                git_ref: Some("main".into()),
            },
            action: Action::Build,
            options: NixOptions::default(),
            origin: Origin::User {
                user: "r3dlust".into(),
                host: "altar".into(),
            },
            now: false,
        };
        let mut client =
            CoeusClient::connect(&ClientSettings::default(), &self.endpoint, &self.key)
                .await
                .unwrap();
        client.deploy(order).await.unwrap().unwrap();
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.cancel();
        self.harness.shutdown.cancel();
    }
}

#[tokio::test]
async fn the_stream_shows_what_a_daemon_reports_to_the_read_only_key() {
    let live = Live::start().await;
    let mut events = Events::open(&live.site.router, "/api/stream", &[]).await;
    assert_eq!(events.next().await.unwrap().event, "snapshot");

    live.order().await;
    let running = events
        .until(|message| {
            message.event == "machine"
                && message.data["machine"]["state"]["running"]["state"]["kind"] == "running"
        })
        .await;
    let lab = &running.last().unwrap().data["machine"];
    assert_eq!(lab["link"]["kind"], "connected");
    assert_eq!(lab["state"]["running"]["id"], 1);

    live.harness.release.notify_one();
    events
        .until(|message| {
            message.event == "machine" && message.data["machine"]["state"]["recent"][0]["id"] == 1
        })
        .await;
}

#[tokio::test]
async fn a_job_that_wrote_no_diff_has_none_to_serve() {
    let live = Live::start().await;
    live.order().await;
    live.harness.release.notify_one();
    live.harness
        .until(|view| view.recent.iter().any(|job| job.id.0 == 1))
        .await;

    let response = get(&live.site.router, "/api/machines/lab/jobs/1/diff", &[]).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body(response).await, "job 1 changed no packages");
}
