use std::net::SocketAddr;
use std::sync::Arc;
use std::{net::IpAddr, path::PathBuf};

use easy_config_store::ConfigStore;
use eyre::Result;
use lazy_static::lazy_static;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};

lazy_static! {
    static ref DEFAULT_SERVER_CONFIG: ConfigInner = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/config.server.default.toml",
    )))
    .unwrap();
}

pub type ServerConfig = Arc<ConfigStore<ConfigInner>>;
pub fn server_config(path: PathBuf) -> Result<ServerConfig> {
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
    #[serde(with = "crate::utils::psk")]
    pub key: [u8; 32],

    host: IpAddr,

    #[serde(default = "super::default_config_path")]
    port: u16,
}

impl ConfigInner {
    pub fn address(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

impl Default for ConfigInner {
    fn default() -> Self {
        let mut config = DEFAULT_SERVER_CONFIG.clone();

        // generate a truly random key to avoid using the default key
        // this should serve as a general safeguard against accidentally
        // using the default key in production
        config.key = rand::random();

        warn!("using default configuration, generating random key");

        config
    }
}
