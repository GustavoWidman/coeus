use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use easy_config_store::ConfigStore;
use eyre::Result;
use log::debug;
use serde::{Deserialize, Serialize};

use super::{DEFAULT_PORT, KeySource};
use crate::common::model::Hostname;

/// the CLI's settings: its key and the fleet inventory
#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct ClientSettings {
    #[serde(flatten)]
    pub key: KeySource,

    /// the machines `@all` and tags select; any other name is reached as a
    /// tailnet hostname on the default port
    #[serde(default)]
    pub hosts: BTreeMap<Hostname, HostEntry>,

    #[serde(with = "humantime_serde", default = "ClientSettings::default_timeout")]
    pub connect_timeout: Duration,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize, Clone)]
pub struct HostEntry {
    /// a hostname or IP address, the entry's name when unset
    #[serde(default)]
    pub address: Option<String>,

    #[serde(default)]
    pub port: Option<u16>,

    #[serde(default)]
    pub tags: Vec<String>,
}

/// where to reach one daemon, resolved again on every connect so MagicDNS
/// changes are picked up
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl ClientSettings {
    const fn default_timeout() -> Duration {
        Duration::from_secs(5)
    }

    /// the explicit path, else the first existing default location, else
    /// built-in defaults with the key taken from the environment
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = match explicit {
            Some(path) => Some(path.to_owned()),
            None => Self::locations().into_iter().find(|path| path.is_file()),
        };

        let Some(path) = path else {
            debug!("no client configuration found, using defaults");
            return Ok(Self::default());
        };

        let store = ConfigStore::<Self>::read(&path, "settings".to_owned())?;
        debug!("loaded configuration from {}", path.display());
        Ok((*store).clone())
    }

    fn locations() -> Vec<PathBuf> {
        let user = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

        user.map(|dir| dir.join("coeus/coeus.toml"))
            .into_iter()
            .chain([PathBuf::from("/etc/coeus/coeus.toml")])
            .collect()
    }

    pub fn endpoint(&self, name: &Hostname) -> Endpoint {
        let entry = self.hosts.get(name);

        Endpoint {
            host: entry
                .and_then(|entry| entry.address.clone())
                .unwrap_or_else(|| name.to_string()),
            port: entry.and_then(|entry| entry.port).unwrap_or(DEFAULT_PORT),
        }
    }
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            key: KeySource::default(),
            hosts: BTreeMap::new(),
            connect_timeout: Self::default_timeout(),
        }
    }
}
