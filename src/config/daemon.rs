use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use easy_config_store::ConfigStore;
use eyre::Result;
use log::debug;
use serde::{Deserialize, Serialize};

use super::{DEFAULT_PORT, KeySource};
use crate::common::model::Hostname;

pub type DaemonConfig = Arc<ConfigStore<DaemonSettings>>;

pub fn daemon_config(path: &Path) -> Result<DaemonConfig> {
    let config = ConfigStore::<DaemonSettings>::read(path, "settings".to_owned())?;
    debug!("loaded configuration from {}", path.display());
    Ok(config.arc())
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct DaemonSettings {
    #[serde(flatten)]
    pub key: KeySource,

    /// where coeusd accepts connections
    #[serde(default = "DaemonSettings::default_listen")]
    pub listen: SocketAddr,

    /// the flake repository holding every machine's configuration, e.g.
    /// `git+https://github.com/owner/nix`
    pub flake: String,

    /// this machine's name in the flake, the system hostname when unset
    #[serde(default)]
    pub hostname: Option<Hostname>,

    #[serde(default = "DaemonSettings::default_state_dir")]
    pub state_dir: PathBuf,

    /// an ssh key for private `git+ssh` flake inputs
    #[serde(default)]
    pub ssh_key_file: Option<PathBuf>,

    /// how many finished jobs to keep
    #[serde(default = "DaemonSettings::default_history")]
    pub history: usize,

    /// a git ref this machine catches up with on its own
    #[serde(default)]
    pub follow: Option<FollowSettings>,

    #[serde(default)]
    pub guard: GuardSettings,

    #[serde(default)]
    pub notify: Option<NotifySettings>,

    /// the program macOS runners start under, which holds their Full Disk
    /// Access grant. coeusd installs it here once from its own package and
    /// never replaces it, so the grant survives coeus updates.
    #[serde(default = "DaemonSettings::default_launcher")]
    pub launcher: PathBuf,
}

impl DaemonSettings {
    fn default_listen() -> SocketAddr {
        SocketAddr::from(([0u16; 8], DEFAULT_PORT))
    }

    fn default_state_dir() -> PathBuf {
        PathBuf::from("/var/lib/coeus")
    }

    const fn default_history() -> usize {
        100
    }

    fn default_launcher() -> PathBuf {
        PathBuf::from("/Library/Application Support/coeus/coeus-launcher")
    }
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            key: KeySource::default(),
            listen: Self::default_listen(),
            flake: String::new(),
            hostname: None,
            state_dir: Self::default_state_dir(),
            ssh_key_file: None,
            history: Self::default_history(),
            follow: None,
            guard: GuardSettings::default(),
            notify: None,
            launcher: Self::default_launcher(),
        }
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct FollowSettings {
    #[serde(rename = "ref")]
    pub git_ref: String,

    /// how often to look for a newer commit
    #[serde(with = "humantime_serde", default = "FollowSettings::default_interval")]
    pub interval: Duration,
}

impl FollowSettings {
    const fn default_interval() -> Duration {
        Duration::from_secs(15 * 60)
    }
}

/// the health checks a machine must pass after activating, or it rolls back
#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct GuardSettings {
    #[serde(default = "GuardSettings::default_enable")]
    pub enable: bool,

    #[serde(with = "humantime_serde", default = "GuardSettings::default_timeout")]
    pub timeout: Duration,

    /// require tailscale to be up, when it is installed
    #[serde(default = "GuardSettings::default_enable")]
    pub tailscale: bool,
}

impl GuardSettings {
    const fn default_enable() -> bool {
        true
    }

    const fn default_timeout() -> Duration {
        Duration::from_secs(120)
    }
}

impl Default for GuardSettings {
    fn default() -> Self {
        Self {
            enable: Self::default_enable(),
            timeout: Self::default_timeout(),
            tailscale: Self::default_enable(),
        }
    }
}

/// where finished jobs are announced, e.g. an ntfy topic or a webhook
#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct NotifySettings {
    pub url: String,

    #[serde(default)]
    pub on: NotifyOn,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum NotifyOn {
    #[default]
    Failure,
    Always,
}
