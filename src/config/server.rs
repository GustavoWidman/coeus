//! Immutable server configuration. Public store TOML references a private runtime PSK.
//! Never generate defaults or serialize resolved secrets back to the source file.
use eyre::{Result, ensure};
use serde::Deserialize;
use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(unix)]
mod files;
#[cfg(test)]
mod tests;

pub type ServerConfig = Arc<ConfigInner>;

pub fn server_config(path: PathBuf) -> Result<ServerConfig> {
    Ok(Arc::new(load(&path)?.1))
}

#[cfg(unix)]
pub(crate) fn files_for_startup(path: &Path, uid: u32) -> Result<(PathBuf, ConfigInner)> {
    files::load(path, uid)
}

/// Shared by native startup and independent activation, before any ledger/bind work.
#[cfg(unix)]
pub(crate) fn load(path: &Path) -> Result<(PathBuf, ConfigInner)> {
    files::load(path, nix::unistd::geteuid().as_raw())
}

#[cfg(not(unix))]
pub(crate) fn load(_path: &Path) -> Result<(PathBuf, ConfigInner)> {
    eyre::bail!("server configuration permission validation requires Unix")
}

#[derive(Deserialize)]
struct Document {
    settings: RawSettings,
}

#[derive(Deserialize)]
struct RawSettings {
    key: Option<String>,
    key_file: Option<PathBuf>,
    host: IpAddr,
    #[serde(default = "super::default_config_path")]
    port: u16,
    repo_url: String,
    #[serde(default = "default_state_directory")]
    state_directory: PathBuf,
}

#[derive(PartialEq, Clone)]
pub struct ConfigInner {
    pub key: [u8; 32],
    host: IpAddr,
    port: u16,
    pub repo_url: String,
    /// Private durable ledger, relative to daemon working directory unless absolute.
    pub state_directory: PathBuf,
}

impl fmt::Debug for ConfigInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl ConfigInner {
    pub fn address(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

fn parse(raw: &str) -> Result<RawSettings> {
    let document: Document =
        toml::from_str(raw).map_err(|_| eyre::eyre!("invalid server TOML (details suppressed)"))?;
    Ok(document.settings)
}

fn decode_key(bytes: &[u8]) -> Result<[u8; 32]> {
    ensure!(
        bytes.len() == 64,
        "PSK must contain exactly 64 hexadecimal characters"
    );
    let mut key = [0; 32];
    for (index, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let hex = |byte| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        };
        let hi = hex(pair[0]).ok_or_else(|| eyre::eyre!("invalid PSK hex (details suppressed)"))?;
        let lo = hex(pair[1]).ok_or_else(|| eyre::eyre!("invalid PSK hex (details suppressed)"))?;
        key[index] = hi << 4 | lo;
    }
    Ok(key)
}

fn default_state_directory() -> PathBuf {
    PathBuf::from(".coeus-state")
}
