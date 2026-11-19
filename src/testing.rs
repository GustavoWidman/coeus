//! a fake machine and an engine around it, for tests across modules

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use eyre::Result;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{
    client::CoeusClient,
    common::{
        model::{
            Action, FdaStatus, Generation, HostInfo, JobEvent, JobId, MachineView, Outcome, Phase,
            Platform,
        },
        proto::Role,
    },
    config::{
        ClientSettings, DaemonSettings, Endpoint, FollowSettings, GuardSettings, Key, PushSettings,
    },
    engine::{Engine, EngineHandle, Host, SystemFacts},
    job::JobDir,
    server::CoeusServer,
};

pub const TOPLEVEL: &str = "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11";

/// a machine whose runners are tasks in this process that build instantly
/// and finish when the test releases them, as `outcome` says
pub struct FakeHost {
    info: HostInfo,
    release: std::sync::Arc<Notify>,
    outcome: Arc<Mutex<Outcome>>,
    spawned: Mutex<Vec<JobId>>,
    /// runners die before taking their job
    stillborn: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Host for FakeHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn supports(&self, action: Action) -> bool {
        action != Action::Boot
    }

    async fn facts(&self) -> Result<SystemFacts> {
        Ok(SystemFacts::default())
    }

    async fn generations(&self) -> Result<Vec<Generation>> {
        Ok(Vec::new())
    }

    async fn spawn_runner(&self, dir: &JobDir) -> Result<()> {
        self.spawned.lock().unwrap().push(dir.id());
        if self.stillborn.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        let dir = dir.clone();
        let release = self.release.clone();
        let outcome = self.outcome.clone();

        let (taken, ready) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut writer = dir.log().writer().unwrap().unwrap();
            let _ = taken.send(());
            writer
                .event(JobEvent::Started {
                    attempt: 1,
                    pid: 0,
                    version: "test".into(),
                })
                .unwrap();
            writer.event(JobEvent::Phase(Phase::Building)).unwrap();
            writer
                .event(JobEvent::Evaluated {
                    drv: TOPLEVEL.parse().unwrap(),
                    toplevel: TOPLEVEL.parse().unwrap(),
                })
                .unwrap();
            release.notified().await;

            let outcome = dir
                .stop_request()
                .unwrap()
                .map_or_else(|| outcome.lock().unwrap().clone(), |stop| stop.outcome());
            writer.finish(outcome).unwrap();
        });
        let _ = ready.await;
        Ok(())
    }

    async fn release_runner(&self, _: JobId) -> Result<()> {
        Ok(())
    }

    async fn probe_fda(&self) -> FdaStatus {
        FdaStatus::NotApplicable
    }

    async fn reboot(&self) -> Result<()> {
        Ok(())
    }
}

pub struct Harness {
    pub engine: EngineHandle<FakeHost>,
    pub release: std::sync::Arc<Notify>,
    pub stillborn: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// how the runners finish when nobody asked them to stop
    pub outcome: Arc<Mutex<Outcome>>,
    pub state: tempfile::TempDir,
    /// cancelled when the engine wants coeusd to exit into a newer version
    pub restart: CancellationToken,
    pub shutdown: CancellationToken,
}

impl Harness {
    pub async fn start(state: tempfile::TempDir) -> Self {
        Self::start_with(state, std::sync::Arc::new(Notify::new())).await
    }

    /// an engine whose runners share `release` with an earlier engine's, as
    /// detached runners outlive the daemon that started them
    pub async fn start_with(state: tempfile::TempDir, release: std::sync::Arc<Notify>) -> Self {
        Self::launch(
            state,
            release,
            "git+https://example.com/nix".into(),
            None,
            None,
        )
        .await
    }

    /// an engine that reports catch-ups by push
    pub async fn pushing(state: tempfile::TempDir, push: PushSettings) -> Self {
        Self::launch(
            state,
            std::sync::Arc::default(),
            "git+https://example.com/nix".into(),
            None,
            Some(push),
        )
        .await
    }

    /// an engine that follows a ref of the repository at `flake`
    pub async fn following(
        state: tempfile::TempDir,
        flake: String,
        follow: FollowSettings,
    ) -> Self {
        Self::launch(state, std::sync::Arc::default(), flake, Some(follow), None).await
    }

    async fn launch(
        state: tempfile::TempDir,
        release: std::sync::Arc<Notify>,
        flake: String,
        follow: Option<FollowSettings>,
        push: Option<PushSettings>,
    ) -> Self {
        let outcome = Arc::new(Mutex::new(Outcome::Succeeded));
        let stillborn = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let host = FakeHost {
            info: HostInfo {
                hostname: "lab".parse().unwrap(),
                platform: Platform::NixOs,
                system: "x86_64-linux".into(),
                version: "test".into(),
            },
            release: release.clone(),
            outcome: outcome.clone(),
            spawned: Mutex::default(),
            stillborn: stillborn.clone(),
        };

        let settings = DaemonSettings {
            flake,
            follow,
            push,
            state_dir: state.path().to_owned(),
            guard: GuardSettings {
                enable: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let restart = CancellationToken::new();
        let shutdown = CancellationToken::new();
        let config = state.path().join("coeusd.toml");
        let engine = Engine::start(host, settings, &config, restart.clone(), shutdown.clone())
            .await
            .unwrap();

        Self {
            engine,
            release,
            stillborn,
            outcome,
            state,
            restart,
            shutdown,
        }
    }

    pub async fn until(&self, done: impl Fn(&MachineView) -> bool) -> MachineView {
        let mut view = self.engine.watch();
        let reached = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if done(&view.borrow_and_update()) {
                    return view.borrow().clone();
                }
                view.changed().await.unwrap();
            }
        })
        .await;

        reached.unwrap_or_else(|_| {
            let last = self.engine.watch().borrow().clone();
            let state =
                |job: &crate::common::model::JobSummary| format!("{} {:?}", job.id, job.state);
            panic!(
                "the engine never reached the expected state; running {:?}, next {:?}, recent {:?}",
                last.running.as_ref().map(|running| state(&running.summary)),
                last.next.as_ref().map(state),
                last.recent.iter().map(state).collect::<Vec<_>>(),
            )
        })
    }
}

/// a flake in a git repository on disk, with a `deployed` branch
pub struct GitFlake {
    pub dir: tempfile::TempDir,
}

impl GitFlake {
    /// a repository with one commit on `deployed`
    pub fn init() -> Self {
        let flake = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        flake.git(&["init", "--quiet", "--initial-branch", "deployed"]);
        std::fs::write(flake.dir.path().join("flake.nix"), "{ outputs = _: { }; }").unwrap();
        flake.git(&["add", "flake.nix"]);
        flake.git(&["commit", "--quiet", "-m", "first"]);
        flake
    }

    /// runs git in the repository, returning what it printed
    pub fn git(&self, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(self.dir.path())
            .args(["-c", "user.name=coeus", "-c", "user.email=coeus@localhost"])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    pub fn url(&self) -> String {
        format!("git+file://{}", self.dir.path().display())
    }

    /// whether nix can look the flake up here, which takes a nix daemon
    pub fn resolvable(&self) -> bool {
        let resolvable = std::process::Command::new("nix")
            .args(["flake", "metadata", "--refresh"])
            .args(["--extra-experimental-features", "nix-command flakes"])
            .arg(format!("{}?ref=deployed", self.url()))
            .env("NIX_REMOTE", "daemon")
            .output()
            .is_ok_and(|output| output.status.success());
        if !resolvable {
            eprintln!("skipped: no nix daemon to resolve a flake with");
        }
        resolvable
    }
}

/// an engine behind a coeus server on loopback, with a full and a read-only
/// key
pub struct Served {
    pub harness: Harness,
    pub endpoint: Endpoint,
    pub key: Key,
    pub read_key: Key,
    pub restart: CancellationToken,
}

impl Served {
    pub async fn start() -> Self {
        Self::serve(Harness::start(tempfile::tempdir().unwrap()).await).await
    }

    pub async fn serve(harness: Harness) -> Self {
        let key = Key::generate();
        let read_key = Key::generate();
        let keys = [(Role::Full, key.clone()), (Role::Read, read_key.clone())];
        let server = CoeusServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            &keys,
            harness.engine.clone(),
        )
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
            read_key,
            restart,
        }
    }

    pub async fn client(&self) -> CoeusClient {
        self.client_with(&self.key).await
    }

    pub async fn reader(&self) -> CoeusClient {
        self.client_with(&self.read_key).await
    }

    pub async fn client_with(&self, key: &Key) -> CoeusClient {
        CoeusClient::connect(&ClientSettings::default(), &self.endpoint, key)
            .await
            .unwrap()
    }

    pub fn stop(&self) {
        self.restart.cancel();
        self.harness.shutdown.cancel();
    }
}

/// a push service on loopback and the push settings that reach it
pub mod push {
    use std::{
        collections::BTreeMap,
        path::Path,
        sync::{Arc, Mutex},
    };

    use axum::{
        Router,
        body::Bytes,
        extract::{Path as Route, State},
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    use gjallar::{Receiver, VapidKey};
    use tokio::sync::watch;

    use crate::config::{Origin, PushSettings};

    pub fn origin() -> Origin {
        "https://coeus.example.com".parse().unwrap()
    }

    pub fn entry(name: &str, receiver: &Receiver, endpoint: &str) -> serde_json::Value {
        let mut json = serde_json::to_value(receiver.subscription(endpoint).unwrap()).unwrap();
        json["name"] = name.into();
        json
    }

    pub fn settings(dir: &Path, key: &VapidKey, entries: &[serde_json::Value]) -> PushSettings {
        let vapid_key_file = dir.join("vapid");
        let subscriptions_file = dir.join("subscriptions.json");
        std::fs::write(&vapid_key_file, format!("{}\n", key.to_base64url())).unwrap();
        std::fs::write(&subscriptions_file, serde_json::to_vec(entries).unwrap()).unwrap();
        PushSettings {
            origin: origin(),
            vapid_key_file,
            subscriptions_file,
            identity_file: None,
        }
    }

    /// what the fake push service saw for each device
    #[derive(Default)]
    pub struct Seen {
        pub payloads: BTreeMap<String, Vec<serde_json::Value>>,
        pub keys: Vec<String>,
        pub topics: Vec<String>,
        /// each message's `Urgency`, in arrival order
        pub urgencies: Vec<String>,
        /// while true, the `held` device gets no answer
        pub hold: watch::Sender<bool>,
    }

    /// a push service on loopback that answers per device: `delivered`, `gone`,
    /// `rotated` (refused under another VAPID key), `busy` (429 every time),
    /// `held` (delivered once [Seen::hold] is false)
    pub async fn fake_push_service(receiver: Receiver) -> (String, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let app = Router::new()
            .route(
                "/push/{device}",
                post(
                    async |State((receiver, seen)): State<(Receiver, Arc<Mutex<Seen>>)>,
                           Route(device): Route<String>,
                           headers: HeaderMap,
                           body: Bytes| {
                        let plaintext = receiver.decrypt(&body).unwrap();
                        let authorization = headers["authorization"].to_str().unwrap().to_owned();
                        let mut held = {
                            let mut seen = seen.lock().unwrap();
                            seen.keys
                                .push(authorization.rsplit_once("k=").unwrap().1.to_owned());
                            seen.topics
                                .push(headers["topic"].to_str().unwrap().to_owned());
                            seen.urgencies
                                .push(headers["urgency"].to_str().unwrap().to_owned());
                            seen.payloads
                                .entry(device.clone())
                                .or_default()
                                .push(serde_json::from_slice(&plaintext).unwrap());
                            seen.hold.subscribe()
                        };
                        if device == "held" {
                            let _ = held.wait_for(|hold| !hold).await;
                        }
                        match device.as_str() {
                            "gone" => (StatusCode::GONE, String::new()),
                            "rotated" => (
                                StatusCode::FORBIDDEN,
                                r#"{"reason":"VapidPkHashMismatch"}"#.to_owned(),
                            ),
                            "busy" => (StatusCode::TOO_MANY_REQUESTS, String::new()),
                            _ => (StatusCode::CREATED, String::new()),
                        }
                    },
                ),
            )
            .with_state((receiver, seen.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (address, seen)
    }
}
