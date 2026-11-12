//! the identity an admin opens every secret with, by default its ssh host
//! key. its public half is world-readable and checked first; the private
//! half is read once through `sudo cat` (sudo prompts on the terminal
//! itself, so Touch ID and the person's pam setup apply) and lives in memory
//! only.

use std::path::Path;

use zeroize::Zeroizing;

use crate::bootstrap::SshKey;

pub struct AdminIdentity {
    public: SshKey,
    identity: age::ssh::Identity,
}

impl AdminIdentity {
    /// `<path>.pub`, no privilege needed
    pub fn public(path: &Path) -> eyre::Result<SshKey> {
        todo!()
    }

    /// `sudo cat <path>` into memory; the caller hands the terminal to sudo
    pub async fn unlock(path: &Path) -> eyre::Result<Self> {
        // TODO: tokio Command sudo cat, stdin inherited, output into
        // Zeroizing<Vec<u8>>; age::ssh::Identity::from_buffer; an encrypted
        // key is refused; its public half must equal public(path)
        todo!()
    }

    pub fn key(&self) -> &SshKey {
        &self.public
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> eyre::Result<Zeroizing<Vec<u8>>> {
        todo!()
    }
}
