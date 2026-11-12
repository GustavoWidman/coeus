//! the per-machine session key: an ed25519 key made on the admin, kept 0600
//! in the state directory, and put in root's `authorized_keys` by the first
//! privileged act. every later ssh is `root@` with it, so a run survives the
//! stock user vanishing in the first switch, or a rollback that loses its
//! password hash. the cleanup step and `--forget` take it out again.
//!
//! its `authorized_keys` line carries the comment `coeus-bootstrap@<host>`,
//! so removing it touches no other line.

use std::path::{Path, PathBuf};

use super::Stock;
use crate::bootstrap::{SshKey, state::StateDir};

pub struct SessionKey {
    private: PathBuf,
    public: SshKey,
}

impl SessionKey {
    /// the key in the state directory, made on first use
    pub fn ensure(state: &StateDir) -> eyre::Result<Self> {
        // TODO: ssh_key::PrivateKey::random(Ed25519) written 0600 through a
        // temporary sibling and a rename, then its `.pub`
        todo!()
    }

    /// the key if a run made one, for cleanup and `--forget`
    pub fn existing(state: &StateDir) -> eyre::Result<Option<Self>> {
        todo!()
    }

    pub fn private(&self) -> &Path {
        &self.private
    }

    pub fn public(&self) -> &SshKey {
        &self.public
    }

    /// in root's `authorized_keys` unless there already
    pub async fn install(&self, stock: &Stock) -> eyre::Result<()> {
        todo!()
    }

    /// out of root's `authorized_keys`; absent already is done
    pub async fn remove(&self, stock: &Stock) -> eyre::Result<()> {
        todo!()
    }
}
