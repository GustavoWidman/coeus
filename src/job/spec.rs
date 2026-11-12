use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{
    common::model::{HostInfo, Hostname, JobId, Order, Platform, Timestamp},
    config::DaemonSettings,
};

/// everything a runner needs to carry out a job, written once at admission.
/// runners never read coeusd's configuration, so a job runs the same way
/// whichever coeusd started it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSpec {
    pub id: JobId,
    pub order: Order,
    pub ordered_at: Timestamp,
    pub target: Target,
    /// an ssh key for private `git+ssh` flake inputs
    pub ssh_key_file: Option<PathBuf>,
    /// the health checks to pass after activating, if the guard is on
    pub guard: Option<Guard>,
    /// install the boot loader while activating, as a machine's first switch
    /// needs when it changes boot loaders
    #[serde(default)]
    pub install_bootloader: bool,
}

impl JobSpec {
    /// the job `settings` admit for `order` on `host`, ordered now
    pub fn admit(id: JobId, order: Order, host: &HostInfo, settings: &DaemonSettings) -> Self {
        Self {
            id,
            order,
            ordered_at: Timestamp::now(),
            target: Target {
                hostname: host.hostname.clone(),
                platform: host.platform,
                flake: settings.flake.clone(),
                sources: settings.state_dir.join("sources"),
            },
            ssh_key_file: settings.ssh_key_file.clone(),
            guard: settings.guard.enable.then(|| Guard {
                timeout: settings.guard.timeout,
                daemon_port: settings.listen.port(),
                tailscale: settings.guard.tailscale,
            }),
            install_bootloader: false,
        }
    }
}

/// the machine a job builds and where its configuration lives
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub hostname: Hostname,
    pub platform: Platform,
    /// the flake repository, e.g. `git+https://github.com/owner/nix`
    pub flake: String,
    /// where uploaded snapshots are unpacked
    pub sources: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guard {
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
    /// the port coeusd listens on, which must answer after activation
    pub daemon_port: u16,
    pub tailscale: bool,
}
