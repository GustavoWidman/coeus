//! the system's `ssh`, so the person's config, agent and jump hosts apply,
//! with what coeus decides:
//!
//! - one control master per remote user under the state directory
//!   (`ControlPersist=120`), reused by every command and by nix's own ssh
//!   (`NIX_SSHOPTS`), so a password login is typed once;
//! - `HostKeyAlgorithms=ssh-ed25519`, so the key ssh verifies is the key
//!   agenix reads;
//! - a `known_hosts` of its own holding exactly the trusted key under a
//!   `HostKeyAlias`, with `StrictHostKeyChecking=yes`, and `BatchMode=yes`
//!   once the first contact is over.

use std::{path::PathBuf, process::ExitStatus};

use super::{Call, SessionKey};
use crate::bootstrap::{SshKey, state::StateDir};

#[derive(Clone, Debug)]
pub struct Ssh {
    destination: String,
    /// `-l root`, overriding the user of the destination or the ssh config
    root: bool,
    /// `-i`, the session key, with `IdentitiesOnly=yes`
    identity: Option<PathBuf>,
    known_hosts: PathBuf,
    control_path: PathBuf,
    /// the host key's name in `known_hosts`, the machine's fleet name
    alias: String,
}

impl Ssh {
    /// as the person's login, against the pinned key
    pub fn login(state: &StateDir, destination: &str) -> Self {
        todo!()
    }

    /// the same machine as root, authenticated by the session key
    pub fn as_root(&self, session: &SessionKey) -> Self {
        todo!()
    }

    /// the key `destination` presents, learned without authenticating:
    /// `PreferredAuthentications=none` against a scratch known_hosts, which
    /// ssh fills before failing at login
    pub async fn first_contact(destination: &str) -> eyre::Result<SshKey> {
        todo!()
    }

    /// trusts `key` for this machine from now on
    pub fn pin(state: &StateDir, key: &SshKey) -> eyre::Result<()> {
        todo!()
    }

    /// the key an earlier run trusted
    pub fn pinned(state: &StateDir) -> eyre::Result<Option<SshKey>> {
        todo!()
    }

    /// opens the control master. without `batch`, ssh may ask a login
    /// password on the terminal the console hands over
    pub async fn connect(&self, batch: bool) -> eyre::Result<()> {
        todo!()
    }

    /// one command on the machine through the master, its argv quoted for
    /// the remote shell
    pub fn command(&self, call: &Call) -> tokio::process::Command {
        todo!()
    }

    /// `ssh-ng://<destination>` and the `NIX_SSHOPTS` that send nix through
    /// the same master and pin
    pub fn nix_store(&self) -> (String, String) {
        todo!()
    }

    /// `ssh -O exit`, so no master outlives the run
    pub async fn close(&self) {
        todo!()
    }

    /// whether a failure was the connection rather than the command: ssh
    /// exits 255 for its own errors
    pub fn lost(status: ExitStatus) -> bool {
        status.code() == Some(255)
    }
}
