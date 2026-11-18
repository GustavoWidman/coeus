use std::{
    num::NonZeroU8,
    path::{Path, PathBuf},
    time::Duration,
};

use eyre::{Result, WrapErr};
use serde::Deserialize;

use super::ClientSettings;

/// `coeus web`'s settings: the inventory it watches, the read-only key it
/// watches with, and where it serves
#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct WebSettings {
    /// `key` or `key_file` holds the read-only key, never the fleet key
    #[serde(flatten)]
    pub client: ClientSettings,

    pub web: WebSection,
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct WebSection {
    /// the unix socket the public site is served on, for cloudflared alone
    pub origin_socket: PathBuf,

    /// the cloudflare tunnel this host connects to while it is ready to
    /// serve; without one nothing reaches the origin socket
    #[serde(default)]
    pub tunnel: Option<TunnelSettings>,

    #[serde(default)]
    pub readiness: ReadinessSettings,
}

/// a tunnel managed here, not in cloudflare's dashboard
#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct TunnelSettings {
    pub id: String,

    /// the credentials `cloudflared tunnel create` wrote for it
    pub credentials_file: PathBuf,

    #[serde(default = "TunnelSettings::default_program")]
    pub cloudflared: PathBuf,
}

impl TunnelSettings {
    fn default_program() -> PathBuf {
        PathBuf::from("cloudflared")
    }
}

/// how often this host checks it can serve, and how many checks in a row
/// change its mind
#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct ReadinessSettings {
    /// also how long one check may take
    #[serde(
        with = "humantime_serde",
        default = "ReadinessSettings::default_interval"
    )]
    pub interval: Duration,

    #[serde(default = "ReadinessSettings::default_up_after")]
    pub up_after: NonZeroU8,

    #[serde(default = "ReadinessSettings::default_down_after")]
    pub down_after: NonZeroU8,
}

impl ReadinessSettings {
    const fn default_interval() -> Duration {
        Duration::from_secs(2)
    }

    const fn default_up_after() -> NonZeroU8 {
        NonZeroU8::new(3).unwrap()
    }

    const fn default_down_after() -> NonZeroU8 {
        NonZeroU8::new(5).unwrap()
    }
}

impl Default for ReadinessSettings {
    fn default() -> Self {
        Self {
            interval: Self::default_interval(),
            up_after: Self::default_up_after(),
            down_after: Self::default_down_after(),
        }
    }
}

impl WebSettings {
    pub const PATH: &'static str = "/etc/coeus/web.toml";

    pub fn load(path: &Path) -> Result<Self> {
        #[derive(Deserialize)]
        struct File {
            settings: WebSettings,
        }

        let text = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read {}", path.display()))?;
        let file: File =
            toml::from_str(&text).wrap_err_with(|| format!("invalid {}", path.display()))?;
        Ok(file.settings)
    }
}
