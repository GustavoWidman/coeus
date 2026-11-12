//! a fake machine and an engine around it, for tests across modules

use std::{sync::Mutex, time::Duration};

use eyre::Result;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{
    common::model::{
        Action, FdaStatus, Generation, HostInfo, JobEvent, JobId, MachineView, Outcome, Phase,
        Platform,
    },
    config::{DaemonSettings, GuardSettings},
    engine::{Engine, EngineHandle, Host, SystemFacts},
    job::JobDir,
};

pub const TOPLEVEL: &str = "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11";

/// a machine whose runners are tasks in this process that build instantly
/// and finish when the test releases them
pub struct FakeHost {
    info: HostInfo,
    release: std::sync::Arc<Notify>,
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
                .map_or(Outcome::Succeeded, |stop| stop.outcome());
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
    pub state: tempfile::TempDir,
    pub shutdown: CancellationToken,
}

impl Harness {
    pub async fn start(state: tempfile::TempDir) -> Self {
        Self::start_with(state, std::sync::Arc::new(Notify::new())).await
    }

    /// an engine whose runners share `release` with an earlier engine's, as
    /// detached runners outlive the daemon that started them
    pub async fn start_with(state: tempfile::TempDir, release: std::sync::Arc<Notify>) -> Self {
        let stillborn = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let host = FakeHost {
            info: HostInfo {
                hostname: "lab".parse().unwrap(),
                platform: Platform::NixOs,
                system: "x86_64-linux".into(),
                version: "test".into(),
            },
            release: release.clone(),
            spawned: Mutex::default(),
            stillborn: stillborn.clone(),
        };

        let settings = DaemonSettings {
            flake: "git+https://example.com/nix".into(),
            state_dir: state.path().to_owned(),
            guard: GuardSettings {
                enable: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let shutdown = CancellationToken::new();
        let config = state.path().join("coeusd.toml");
        let engine = Engine::start(
            host,
            settings,
            &config,
            CancellationToken::new(),
            shutdown.clone(),
        )
        .await
        .unwrap();

        Self {
            engine,
            release,
            stillborn,
            state,
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
