use std::path::{Path, PathBuf};

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
