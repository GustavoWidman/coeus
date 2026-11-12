//! putting every secret on exactly its recipients: new ones encrypted from
//! memory, stale ones re-encrypted with the admin's identity

use std::path::{Path, PathBuf};

use super::{AdminIdentity, Header, Verdict};
use crate::bootstrap::{
    SshKey,
    contract::{RepoPath, Rules},
};

/// every managed secret of the clone and how it stands
#[derive(Clone, Debug)]
pub struct Plan {
    root: PathBuf,
    pub files: Vec<Planned>,
    /// `.age` files the contract does not manage, left alone
    pub unmanaged: Vec<RepoPath>,
}

#[derive(Clone, Debug)]
pub struct Planned {
    pub path: RepoPath,
    pub recipients: Vec<SshKey>,
    pub header: Option<Header>,
    pub verdict: Verdict,
}

/// reads the header of every file `rules` names, under `root`; no key needed
pub fn plan(root: &Path, rules: &Rules) -> eyre::Result<Plan> {
    todo!()
}

impl Plan {
    /// the files whose header disagrees with their recipients
    pub fn stale(&self) -> impl Iterator<Item = &Planned> {
        self.files
            .iter()
            .filter(|file| matches!(file.verdict, Verdict::Stale { .. }))
    }

    /// the stale files whose header does not name `admin`, which it could
    /// not decrypt: checked before any sudo prompt
    pub fn unopenable_by(&self, admin: &SshKey) -> Vec<RepoPath> {
        todo!()
    }

    /// re-encrypts each stale file for its recipients, keeping armor as it
    /// was. a run stopped between two files leaves each one old or new,
    /// never half, and the next run rekeys the rest. returns what it rewrote.
    pub fn apply(
        &self,
        identity: &AdminIdentity,
        progress: impl FnMut(usize, usize, &RepoPath),
    ) -> eyre::Result<Vec<RepoPath>> {
        // TODO: for each stale file: identity.decrypt(bytes) into Zeroizing,
        // encrypt to its recipients (armored when header.armored), replace()
        todo!()
    }
}

/// encrypts a new secret to `recipients` and puts it at `file`, binary
pub fn create(file: &Path, plaintext: &[u8], recipients: &[SshKey]) -> eyre::Result<()> {
    todo!()
}

/// writes through a temporary sibling, fsync and a rename
fn replace(file: &Path, bytes: &[u8]) -> eyre::Result<()> {
    todo!()
}
