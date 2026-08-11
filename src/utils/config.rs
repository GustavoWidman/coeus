use std::net::SocketAddr;
use std::sync::Arc;
use std::{net::IpAddr, path::PathBuf};

use easy_config_store::ConfigStore;
use eyre::Result;
use lazy_static::lazy_static;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};

lazy_static! {
    static ref DEFAULT_CONFIG: ConfigInner = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/config.default.toml",
    )))
    .unwrap();
}

pub type Config = Arc<ConfigStore<ConfigInner>>;
pub fn config(path: PathBuf) -> Result<Config> {
    let config = ConfigStore::<ConfigInner>::read(path, "settings".to_string())?;

    info!("config parsing successful");
    debug!(
        "loaded configuration:\n{}",
        toml::to_string_pretty(&*config)?
    );

    Ok(config.arc())
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct ConfigInner {
    host: IpAddr,
    port: u16,
}

impl ConfigInner {
    pub fn address(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

impl Default for ConfigInner {
    fn default() -> Self {
        let mut config = DEFAULT_CONFIG.clone();

        config
    }
}
