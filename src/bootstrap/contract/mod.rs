//! what a configuration repository tells coeus about adding a machine,
//! from a plain Nix file (`.coeus/bootstrap.nix` by default). coeus knows no
//! repository layout: which files a machine adds, what to ask, which secrets
//! it makes and who opens each secret all come from here.
//!
//! every call is one `nix eval --json --file <contract> --apply '<fn>'`, the
//! input embedded in the function as a string literal: no flake is
//! evaluated, no input fetched, no temporary file written and no
//! `--impure` needed. the file is read from the clone as it is now, so what
//! a step wrote is what the next call sees.

mod enrollment;
mod input;
mod question;
pub mod wire;

#[cfg(test)]
mod test;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub use enrollment::{Enrollment, Generator, RepoPath};
pub use input::{Input, nix_string};
pub use question::{Answers, Question, QuestionKind};
pub use wire::{Choice, FirstSwitch, Messages};

use crate::bootstrap::SshKey;

/// where a repository keeps its contract, from its root
pub const DEFAULT_PATH: &str = ".coeus/bootstrap.nix";

/// the contract version this coeus speaks
pub const VERSION: u32 = 1;

/// each secret's recipients; `None` for a `.age` file the repository does
/// not manage
pub type Rules = BTreeMap<RepoPath, Option<Vec<SshKey>>>;

pub struct Contract {
    /// the contract file, absolute
    file: PathBuf,
    identity: PathBuf,
}

impl Contract {
    /// reads `version` and `identity`. a repository without the file, or
    /// with a version this coeus does not speak, is refused saying what to do
    pub async fn open(repo_root: &Path, path: &Path) -> eyre::Result<Self> {
        // TODO: file = repo_root.join(path); absent → error naming DEFAULT_PATH
        // and the readme's contract section;
        // eval::<wire::Head>("c: { inherit (c) version identity; }");
        // version != VERSION → say whether coeus or the repository is older
        todo!()
    }

    /// the identity an admin opens every secret with, such as
    /// `/etc/ssh/ssh_host_ed25519_key`
    pub fn identity(&self) -> &Path {
        &self.identity
    }

    /// every question for this input; the caller asks the first one whose id
    /// is not in `input.answers` and calls again, since a question may
    /// depend on an earlier answer
    pub async fn questions(&self, input: &Input) -> eyre::Result<Vec<Question>> {
        // TODO: eval::<Vec<wire::Question>>(&format!("c: c.questions ({})", input.to_nix()?)),
        // each into a Question
        todo!()
    }

    pub async fn enroll(&self, input: &Input) -> eyre::Result<Enrollment> {
        todo!()
    }

    /// each path's recipients, in one eval for all of them
    pub async fn recipients(&self, paths: &[RepoPath]) -> eyre::Result<Rules> {
        // TODO: the paths as a JSON list in a nix string; apply
        // `c: builtins.listToAttrs (map (p: { name = p; value = c.recipients p; }) paths)`
        // into wire::Recipients; every key parsed with SshKey::parse; a path
        // missing from the answer is an error
        todo!()
    }

    /// `nix eval --json --file <file> --apply <apply>`. a contract that throws
    /// is talking to the person ("write hosts/<name>/boot.nix by hand"), so
    /// the error keeps the last lines of nix's message
    async fn eval<T: serde::de::DeserializeOwned>(&self, apply: &str) -> eyre::Result<T> {
        todo!()
    }
}
