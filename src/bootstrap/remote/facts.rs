//! what the machine is, as the contract receives it under `facts`. one type
//! for the probe that fills it, the contract that reads it and the person who
//! sees it; its JSON is part of the contract, so its fields change only with
//! the contract's version.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::common::model;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Facts {
    pub platform: Platform,
    pub system: System,
    /// the NixOS release (`26.05`) or the macOS version (`26.6.2`)
    pub os_version: String,
    /// the running system's `system.stateVersion`; `None` on a Mac
    pub state_version: Option<String>,
    /// what the machine calls itself now, which may not be its fleet name
    pub hostname: String,
    pub efi: bool,
    pub boot_loader: Option<BootLoader>,
    /// the device grub installs to
    pub boot_device: Option<String>,
    /// `nixos-generate-config --show-hardware-config`; `None` on a Mac
    pub hardware_config: Option<String>,
    /// an IANA name, e.g. `America/Sao_Paulo`
    pub time_zone: Option<String>,
    /// the ssh login coeus reached the machine as
    pub login_user: String,
    /// on a Mac the console user, else the login user; on NixOS the login
    /// user unless that is root
    pub main_user: Option<String>,
    pub cpus: u32,
    pub memory_mib: u64,
    /// `nix --version`'s version, when nix is there
    pub nix_version: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Nixos,
    Darwin,
}

impl From<Platform> for model::Platform {
    fn from(platform: Platform) -> Self {
        match platform {
            Platform::Nixos => Self::NixOs,
            Platform::Darwin => Self::Darwin,
        }
    }
}

/// the nix system double
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum System {
    #[serde(rename = "x86_64-linux")]
    X86_64Linux,
    #[serde(rename = "aarch64-linux")]
    Aarch64Linux,
    #[serde(rename = "aarch64-darwin")]
    Aarch64Darwin,
    #[serde(rename = "x86_64-darwin")]
    X86_64Darwin,
}

impl System {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X86_64Linux => "x86_64-linux",
            Self::Aarch64Linux => "aarch64-linux",
            Self::Aarch64Darwin => "aarch64-darwin",
            Self::X86_64Darwin => "x86_64-darwin",
        }
    }
}

impl fmt::Display for System {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BootLoader {
    SystemdBoot,
    Grub,
    Limine,
}
