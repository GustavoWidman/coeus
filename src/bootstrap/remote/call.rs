//! every command coeus runs on a machine, so none is built from a string
//! anywhere else. `argv` is pure and every variant's is pinned by a test.

use std::path::PathBuf;

use crate::{
    bootstrap::SshKey,
    common::model::{Hostname, NarHash, OrderId, StorePath},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `sh -s` reading probe.sh on stdin, its `key=value` lines on stdout
    Probe,
    /// `nixos-generate-config --show-hardware-config`, as root
    HardwareConfig,
    /// Determinate's installer, as root, without questions
    InstallNix,
    /// `/nix/nix-installer repair`, for an install a crash cut short
    RepairNix,
    /// moves `/etc/bashrc`, `/etc/zshrc` and `/etc/zprofile` to
    /// `.before-nix-darwin` when nix-darwin did not write them; prints the
    /// ones it moved
    MoveEtcAside,
    /// adds the session key to root's `authorized_keys` unless it is there
    InstallSessionKey { key: SshKey, host: Hostname },
    /// removes every line carrying the session key
    RemoveSessionKey { key: SshKey },
    /// `nix build --json --log-format internal-json <flake>#<attribute>` as
    /// root, the coeus the machine's configuration pins
    BuildCoeus {
        source: StorePath,
        attribute: String,
    },
    /// `<coeus>/bin/coeusd --version`
    CoeusVersion { coeus: StorePath },
    /// `<coeus>/bin/coeusd adopt …`, as root
    Adopt(AdoptCall),
    /// `<coeus>/bin/coeusd adopt --status --nar-hash …`, as root
    AdoptStatus { coeus: StorePath, nar_hash: NarHash },
    /// `tailscale status --json`, when coeusd stays silent
    Tailscale,
}

/// the flags of `coeusd adopt`, the one interface between this CLI and the
/// fleet's coeus before a coeusd runs: kept small, it outlives version skew
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptCall {
    pub coeus: StorePath,
    pub source: PathBuf,
    pub nar_hash: NarHash,
    pub label: String,
    pub hostname: Hostname,
    pub order: OrderId,
    pub install_bootloader: bool,
    /// stream the job's events from this log offset
    pub follow: Option<u64>,
}

impl Call {
    pub fn argv(&self) -> Vec<String> {
        todo!()
    }

    /// whether it must run as root
    pub fn privileged(&self) -> bool {
        todo!()
    }

    /// what the call reads on stdin, beyond a sudo password
    pub fn stdin(&self) -> Option<&'static [u8]> {
        match self {
            Self::Probe => Some(include_bytes!("probe.sh")),
            _ => None,
        }
    }
}
