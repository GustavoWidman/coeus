//! the enrollment lives on `coeus/bootstrap/<host>`, in a private clone under
//! the bootstrap's state directory (`git clone --reference-if-able <the
//! person's checkout> <its origin>`), so the person's checkout, its branch,
//! its uncommitted work and its jj state are never touched.
//!
//! the branch is the bootstrap's durable record, two commits on a freshly
//! fetched `origin/<default>`:
//!
//! ```text
//! <the contract's enroll subject>
//!
//! Coeus-Bootstrap: enroll <host>
//!
//! <the contract's rekey subject>       ← always on top, derived: dropped
//!                                         and made again after a rebase
//! Coeus-Bootstrap: rekey <host>
//! ```
//!
//! a run on this admin or another one reads what was done from the branch.

pub mod body;
mod forge;

#[cfg(test)]
mod test;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub use forge::{GhError, PullRequest, PullState};

use crate::{
    bootstrap::{contract::RepoPath, state::StateDir},
    common::model::{CommitSha, Hostname},
};

pub struct Branch {
    host: Hostname,
    clone: PathBuf,
    /// origin's default branch, from `refs/remotes/origin/HEAD`
    base: String,
}

/// the enroll commit, as the branch shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enrolled {
    pub commit: CommitSha,
    pub subject: String,
    /// the paths it added
    pub files: Vec<RepoPath>,
}

impl Branch {
    pub fn name(host: &Hostname) -> String {
        format!("coeus/bootstrap/{host}")
    }

    /// the clone, made or repaired from whatever is there:
    ///
    /// - no clone at [StateDir::clone_dir], or a half-made one: cloned again
    /// - the branch only on origin (another admin started it): tracked
    /// - no branch anywhere: a new one on a freshly fetched `origin/<base>`
    /// - a stale `index.lock`: removed, since the bootstrap's flock proves no
    ///   git of ours runs there
    pub async fn open(
        state_dir: &StateDir,
        person_checkout: &Path,
        host: &Hostname,
    ) -> eyre::Result<Self> {
        todo!()
    }

    pub fn clone_dir(&self) -> &Path {
        &self.clone
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn head(&self) -> eyre::Result<CommitSha> {
        todo!()
    }

    /// the enroll commit, if the branch has one
    pub async fn enrolled(&self) -> eyre::Result<Option<Enrolled>> {
        todo!()
    }

    /// the declared paths that already exist on `origin/<base>`: the machine
    /// is enrolled there
    pub async fn on_base(&self, paths: &[&RepoPath]) -> eyre::Result<Vec<RepoPath>> {
        todo!()
    }

    pub fn path(&self, path: &RepoPath) -> PathBuf {
        self.clone.join(path.as_str())
    }

    /// writes the files that are absent, never edits one that exists, and
    /// stages every declared path; returns the ones it wrote
    pub async fn write(&self, files: &BTreeMap<RepoPath, String>) -> eyre::Result<Vec<RepoPath>> {
        todo!()
    }

    /// commits exactly `paths` with the enroll trailer
    pub async fn commit_enroll(&self, subject: &str, paths: &[RepoPath]) -> eyre::Result<Enrolled> {
        todo!()
    }

    /// fetches origin; the commits `origin/<base>` has that the branch lacks
    pub async fn behind(&self) -> eyre::Result<u32> {
        todo!()
    }

    /// drops the rekey commit and rebases the rest onto `origin/<base>`. a
    /// conflict aborts the rebase and blocks the run with the clone's path.
    pub async fn rebase_dropping_rekey(&self) -> eyre::Result<()> {
        todo!()
    }

    /// paths with changes nobody committed
    pub async fn uncommitted(&self) -> eyre::Result<Vec<String>> {
        todo!()
    }

    /// every tracked `.age` file
    pub async fn secrets(&self) -> eyre::Result<Vec<RepoPath>> {
        todo!()
    }

    /// the rekeyed files as the top commit, replacing an older rekey commit
    pub async fn commit_rekey(
        &self,
        subject: &str,
        rekeyed: &[RepoPath],
    ) -> eyre::Result<CommitSha> {
        todo!()
    }

    /// `git push --force-with-lease` of the head, against the remote head it
    /// saw, since a rebase rewrites; nothing when origin has it already
    pub async fn push(&self) -> eyre::Result<()> {
        todo!()
    }
}
