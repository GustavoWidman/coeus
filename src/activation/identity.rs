use eyre::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryIdentity {
    pub version: String,
    pub protocol: u8,
    pub os: String,
    pub arch: String,
}
impl BinaryIdentity {
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: crate::common::proto::PROTOCOL_VERSION,
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
        }
    }
}

/// Declared account lifecycle policy, including numeric IDs when managed/pinned.
/// Unknown automatically allocated Linux IDs are absent, never guessed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountPolicy {
    pub managed: bool,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

/// Immutable, module-generated description. No credentials enter this file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceIdentity {
    pub host: String,
    pub platform: String,
    pub daemon: PathBuf,
    pub version: String,
    pub protocol: u8,
    pub user: String,
    pub account: AccountPolicy,
    pub state_directory: PathBuf,
    pub config_file: PathBuf,
    /// Immutable initial service executable (native daemon on Darwin, wrapper on Linux).
    /// The full service_definition also captures arguments, environment and policy.
    pub runner: PathBuf,
    pub elevation_adapter: PathBuf,
    pub nix: PathBuf,
    /// Exact generated service definition (unit/plist), including overrides.
    #[serde(default)]
    pub service_definition: String,
}
impl ServiceIdentity {
    pub fn read(closure: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(closure.join("coeus-service.toml"))?;
        let mut identity: Self = toml::from_str(&text)?;
        identity.validate()?;
        let relative = match identity.platform.as_str() {
            "linux" => "etc/systemd/system/coeusd.service",
            "darwin" => "Library/LaunchDaemons/org.nixos.coeusd.plist",
            _ => {
                bail!("unsupported activation platform");
            }
        };
        identity.service_definition = std::fs::read_to_string(closure.join(relative))?;
        ensure!(
            !identity.service_definition.is_empty(),
            "empty service definition"
        );
        Ok(identity)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.host.is_empty()
                && self.host.len() < 128
                && self
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid target host"
        );
        ensure!(
            matches!(self.platform.as_str(), "linux" | "darwin"),
            "unsupported platform"
        );
        ensure!(
            self.daemon.starts_with("/nix/store")
                && self.runner.starts_with("/nix/store")
                && self.nix.starts_with("/nix/store")
                && self.elevation_adapter.starts_with("/nix/store"),
            "service executable must be immutable"
        );
        ensure!(
            self.account.managed || (self.account.uid.is_none() && self.account.gid.is_none()),
            "unmanaged account must not declare managed numeric IDs"
        );
        ensure!(
            !self.account.managed
                || self.platform != "darwin"
                || (self.account.uid.is_some_and(|uid| uid > 0) && self.account.gid.is_some()),
            "managed Darwin account requires explicit numeric UID/GID"
        );
        ensure!(
            self.state_directory.is_absolute(),
            "absolute service working directory required"
        );
        ensure!(
            self.protocol == crate::common::proto::PROTOCOL_VERSION,
            "target protocol incompatible with operator"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaemonIdentity {
    pub binary: BinaryIdentity,
    pub host: String,
    pub executable: PathBuf,
    pub runner: PathBuf,
    pub process_id: u32,
}
pub fn daemon_identity() -> Result<DaemonIdentity> {
    let manifest = std::env::var_os("COEUS_SERVICE_MANIFEST").ok_or_else(|| {
        eyre::eyre!("daemon is not running under a supported Coeus service module")
    })?;
    let identity: ServiceIdentity = toml::from_str(&std::fs::read_to_string(manifest)?)?;
    identity.validate()?;
    Ok(DaemonIdentity {
        binary: BinaryIdentity::current(),
        host: identity.host,
        executable: std::fs::canonicalize(std::env::current_exe()?)?,
        runner: identity.runner,
        process_id: std::process::id(),
    })
}
