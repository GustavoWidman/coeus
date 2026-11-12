use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::common::model::{Hostname, JobId, Order, Platform, Timestamp};

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
