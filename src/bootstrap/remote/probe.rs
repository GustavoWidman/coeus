//! what probe.sh found, parsed into types at the boundary, so a probe that
//! changes shape fails here and not in a contract template

use super::{Facts, NixState};
use crate::bootstrap::SshKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probed {
    pub facts: Facts,
    /// what sshd serves from `/etc/ssh/ssh_host_ed25519_key.pub`, which must
    /// be the pinned key
    pub host_key: SshKey,
    /// nix on a Mac; always installed on NixOS
    pub nix: NixState,
}

impl Probed {
    /// probe.sh's `key=value` lines, plus the hardware configuration read as
    /// root on NixOS
    pub fn parse(output: &str, hardware_config: Option<String>) -> eyre::Result<Self> {
        // TODO: one field per key; an unknown platform or system is refused
        // naming what the probe saw (coeus only adopts NixOS and macOS), a
        // missing key is an error naming it; mainUser per the contract's
        // rules (console user on a Mac, the login user unless root on NixOS)
        todo!()
    }
}

impl Facts {
    /// `NixOS 26.05 · x86_64-linux · EFI · 16 GiB · 8 cpus`, the reach row
    pub fn describe(&self) -> String {
        todo!()
    }
}
