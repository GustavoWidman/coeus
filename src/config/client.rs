//! Client credentials and fleet settings. Loading never creates or rewrites credentials.
use eyre::{Result, WrapErr, ensure, eyre};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

pub type ClientConfig = Arc<ConfigInner>;

pub fn default_client_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path).join("coeus/config.toml"));
    }
    Ok(PathBuf::from(
        std::env::var_os("HOME").ok_or_else(|| eyre!("HOME unavailable; pass --config"))?,
    )
    .join(".config/coeus/config.toml"))
}

pub fn client_config(path: PathBuf) -> Result<ClientConfig> {
    #[derive(Deserialize)]
    struct File {
        settings: ConfigInner,
        #[serde(default)]
        bootstrap: BTreeMap<String, BootstrapConfig>,
    }
    let text = std::fs::read_to_string(&path)
        .wrap_err_with(|| format!("read client config {}", path.display()))?;
    // TOML errors can embed source lines (including the PSK). Never surface their display.
    let mut file: File = toml::from_str(&text).map_err(|_| {
        eyre!("invalid client configuration (details suppressed to protect credentials)")
    })?;
    ensure!(
        !file.settings.default_revision.trim().is_empty(),
        "default_revision must not be empty"
    );
    ensure!(
        file.settings.fleet.len() <= 128,
        "fleet exceeds 128 targets"
    );
    file.settings.bootstrap = file.bootstrap;
    Ok(Arc::new(file.settings))
}

#[derive(PartialEq, Serialize, Deserialize, Clone)]
pub struct ConfigInner {
    #[serde(with = "crate::utils::psk")]
    pub key: [u8; 32],
    #[serde(default)]
    pub fleet: Vec<String>,
    #[serde(default = "default_revision")]
    pub default_revision: String,
    #[serde(skip)]
    pub bootstrap: BTreeMap<String, BootstrapConfig>,
}
fn default_revision() -> String {
    "main".into()
}
// Deliberately no Debug: neither derived nor nested error logging may expose the PSK.

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct BootstrapConfig {
    pub activation_directory: PathBuf,
    pub peer_config: PathBuf,
    pub flake: String,
    pub hostname: String,
    pub nix_binary: PathBuf,
    #[serde(default)]
    pub operator_sudo: bool,
    pub copy_binary: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_fleet_without_rewriting_or_default_credentials() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config");
        assert!(client_config(path.clone()).is_err());
        assert!(!path.exists());
        let source = format!(
            "[settings]\nkey='{}'\nfleet=['me','lab:123']\n",
            "42".repeat(32)
        );
        std::fs::write(&path, &source)?;
        let config = client_config(path.clone())?;
        assert_eq!(config.default_revision, "main");
        assert_eq!(config.fleet, ["me", "lab:123"]);
        assert_eq!(std::fs::read_to_string(path)?, source);
        Ok(())
    }
    #[test]
    fn parse_errors_do_not_disclose_secret_source() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config");
        std::fs::write(&path, "[settings]\nkey='private-secret'\n")?;
        let error = client_config(path)
            .err()
            .ok_or_else(|| eyre!("expected error"))?;
        assert!(!format!("{error:?}").contains("private-secret"));
        Ok(())
    }
}
