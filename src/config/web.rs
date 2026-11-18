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
    /// where browsers load the app from, `https://coeus.example.com`
    pub public_origin: String,
    /// the unix socket the public site is served on, for cloudflared alone
    pub origin_socket: PathBuf,
    /// the fleet's session keys, 64 hex digits each, one per line; the
    /// first signs, all check, and every serving host holds the same file
    pub session_key_file: PathBuf,
    /// how long a session lasts after its last sign-in on the tailnet
    #[serde(default = "WebSection::session_days")]
    pub session_days: u32,
    pub signin: SigninSettings,

    /// the cloudflare tunnel this host connects to while it is ready to
    /// serve; without one nothing reaches the origin socket
    #[serde(default)]
    pub tunnel: Option<TunnelSettings>,

    #[serde(default)]
    pub readiness: ReadinessSettings,
}

/// signing in, served on this machine's tailnet addresses alone
#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct SigninSettings {
    #[serde(default = "SigninSettings::port")]
    pub port: u16,
    /// this machine's `*.ts.net` certificate and key, as PEM files
    pub cert: PathBuf,
    pub key: PathBuf,
    /// every serving host's sign-in address, `https://<host>.<tailnet>.ts.net:<port>`,
    /// which the app tries in turn
    pub urls: Vec<String>,
    /// the app capability a tailnet policy grant gives a node that may sign in
    #[serde(default = "SigninSettings::capability")]
    pub capability: String,
    /// logins that may sign in without the grant
    #[serde(default)]
    pub logins: Vec<String>,
}

impl WebSection {
    fn session_days() -> u32 {
        7
    }
}

impl SigninSettings {
    fn port() -> u16 {
        8443
    }

    fn capability() -> String {
        "r3dlust.com/cap/coeus".into()
    }
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
