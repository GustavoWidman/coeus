use std::path::{Path, PathBuf};

/// notices when the system now carries a different coeusd or configuration
/// than this process started with, so it can exit into the new one once idle
pub(super) struct UpgradeWatch {
    exe: Option<PathBuf>,
    config: PathBuf,
    config_bytes: Option<Vec<u8>>,
}

impl UpgradeWatch {
    const INSTALLED: &'static str = "/run/current-system/sw/bin/coeusd";

    pub fn new(config: &Path) -> Self {
        Self {
            exe: std::env::current_exe().and_then(std::fs::canonicalize).ok(),
            config: config.to_owned(),
            config_bytes: std::fs::read(config).ok(),
        }
    }

    pub fn stale(&self) -> bool {
        let installed = std::fs::canonicalize(Self::INSTALLED).ok();
        let binary =
            matches!((&installed, &self.exe), (Some(installed), Some(exe)) if installed != exe);
        let config = std::fs::read(&self.config).ok() != self.config_bytes;
        binary || config
    }
}
