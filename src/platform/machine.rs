use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use eyre::{Result, WrapErr, bail};
use log::warn;
use tokio::process::Command;

use super::{
    SystemFacts, SystemRoot,
    launcher::{Launcher, Task},
};
use crate::{
    common::model::{Action, FdaStatus, Generation, HostInfo, JobId, Platform},
    config::DaemonSettings,
    job::JobDir,
};

/// the machine coeusd runs on: what it is, what it runs, and how coeusd
/// starts the runners that change it
#[derive(Clone, Debug)]
pub struct Machine {
    host: HostInfo,
    system: SystemRoot,
    launcher: Launcher,
    state_dir: PathBuf,
    /// this coeusd's own binary, so a job runs on the version that admitted it
    exe: PathBuf,
}

impl Machine {
    const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

    pub fn new(settings: &DaemonSettings) -> Result<Self> {
        let host = HostInfo::detect(settings.hostname.clone())?;
        let launcher = match host.platform {
            Platform::NixOs => Launcher::Systemd,
            Platform::Darwin => Launcher::Launchd {
                program: settings.launcher.clone(),
                plists: settings.state_dir.join("launchd"),
            },
        };

        Ok(Self {
            host,
            system: SystemRoot::default(),
            launcher,
            state_dir: settings.state_dir.clone(),
            exe: std::env::current_exe()?
                .canonicalize()
                .wrap_err("failed to locate the coeusd binary")?,
        })
    }

    pub const fn host(&self) -> &HostInfo {
        &self.host
    }

    pub const fn supports(&self, action: Action) -> bool {
        self.host.platform.supports(action)
    }

    pub async fn facts(&self) -> Result<SystemFacts> {
        Ok(self.system.facts(self.host.platform))
    }

    /// newest first
    pub async fn generations(&self) -> Result<Vec<Generation>> {
        self.system.generations()
    }

    /// starts `coeusd run <dir>` detached from coeusd; a runner already running
    /// for the job is left alone
    pub async fn spawn_runner(&self, dir: &JobDir) -> Result<()> {
        let command = self.command("run", dir.path().as_os_str().to_owned());
        self.launcher
            .start(
                Task::Job(dir.id()),
                &command,
                &dir.path().join("runner.log"),
            )
            .await
    }

    pub async fn release_runner(&self, id: JobId) -> Result<()> {
        self.launcher.release(Task::Job(id)).await
    }

    /// whether runners can activate nix-darwin, asked of a probe started the
    /// way runners are, so it holds exactly the grant they would
    pub async fn probe_fda(&self) -> FdaStatus {
        if self.host.platform != Platform::Darwin {
            return FdaStatus::NotApplicable;
        }

        match self.probe().await {
            Ok(true) => FdaStatus::Granted,
            Ok(false) => FdaStatus::Missing,
            Err(error) => {
                warn!("failed to probe Full Disk Access: {error:#}");
                FdaStatus::Missing
            }
        }
    }

    pub async fn reboot(&self) -> Result<()> {
        let (program, args): (&str, &[&str]) = match self.host.platform {
            Platform::NixOs => ("systemctl", &["reboot"]),
            Platform::Darwin => ("/sbin/shutdown", &["-r", "now"]),
        };

        let status = Command::new(program).args(args).status().await?;
        if !status.success() {
            bail!("{program} failed with {status}");
        }
        Ok(())
    }

    async fn probe(&self) -> Result<bool> {
        let verdict = self.state_dir.join("fda");
        match fs::remove_file(&verdict) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }

        let command = self.command("probe", verdict.as_os_str().to_owned());
        let log = self.state_dir.join("launchd/probe.log");
        self.launcher.start(Task::Probe, &command, &log).await?;

        let started = Instant::now();
        let answer = loop {
            if let Ok(answer) = fs::read_to_string(&verdict) {
                break Ok(answer.trim() == "granted");
            }
            if started.elapsed() > Self::PROBE_TIMEOUT {
                break Err(eyre::eyre!(
                    "the probe did not answer within {:?}",
                    Self::PROBE_TIMEOUT
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        self.launcher.release(Task::Probe).await?;
        answer
    }

    fn command(&self, subcommand: &str, argument: OsString) -> Vec<OsString> {
        vec![self.exe.clone().into(), subcommand.into(), argument]
    }
}
