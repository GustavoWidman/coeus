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
mod eval;
mod input;
mod question;
pub mod wire;

#[cfg(test)]
mod test;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use eyre::{WrapErr, bail, eyre};

pub use enrollment::{Enrollment, Generator, RepoPath};
pub use eval::Failure;
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
        let file = repo_root.join(path);
        if !file.is_file() {
            bail!(
                "{} has no bootstrap contract at {}: the repository says in it how \
                 coeus adds a machine (see \"the bootstrap contract\" in coeus's readme)",
                repo_root.display(),
                path.display(),
            );
        }
        let file = file
            .canonicalize()
            .wrap_err_with(|| format!("failed to read {}", file.display()))?;

        let head: wire::Head = json(&file, "c: { inherit (c) version identity; }").await?;
        if head.version != VERSION {
            let fix = match head.version > VERSION {
                true => "update coeus",
                false => "update the contract",
            };
            bail!(
                "{} is contract version {}, and this coeus speaks version {VERSION}: {fix}",
                path.display(),
                head.version,
            );
        }
        if head.identity.is_empty() {
            bail!("{} names no identity", path.display());
        }

        Ok(Self {
            file,
            identity: head.identity.into(),
        })
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
        let questions: Vec<wire::Question> = self
            .eval(&format!("c: c.questions {}", input.to_nix()?))
            .await?;
        let mut ids = std::collections::BTreeSet::new();
        questions
            .into_iter()
            .map(|question| {
                if !ids.insert(question.id.clone()) {
                    bail!("the contract asks {:?} twice", question.id);
                }
                Question::try_from(question)
            })
            .collect()
    }

    pub async fn enroll(&self, input: &Input) -> eyre::Result<Enrollment> {
        let enrollment: wire::Enrollment = self
            .eval(&format!("c: c.enroll {}", input.to_nix()?))
            .await?;
        enrollment.try_into()
    }

    /// each path's recipients, in one eval for all of them
    pub async fn recipients(&self, paths: &[RepoPath]) -> eyre::Result<Rules> {
        let list: Vec<&str> = paths.iter().map(RepoPath::as_str).collect();
        let apply = format!(
            "c: builtins.listToAttrs (map (p: {{ name = p; value = c.recipients p; }}) {})",
            input::from_json(&serde_json::to_string(&list)?),
        );
        let answer: wire::Recipients = self.eval(&apply).await?;

        paths
            .iter()
            .map(|path| {
                let keys = answer
                    .get(path.as_str())
                    .ok_or_else(|| eyre!("the contract said nothing of {path}"))?;
                let keys = keys
                    .as_ref()
                    .map(|keys| {
                        keys.iter()
                            .map(|key| SshKey::parse(key))
                            .collect::<eyre::Result<Vec<_>>>()
                    })
                    .transpose()
                    .wrap_err_with(|| format!("the recipients of {path}"))?;
                Ok((path.clone(), keys))
            })
            .collect()
    }

    /// `nix eval --json --file <file> --apply <apply>`. a contract that throws
    /// is talking to the person ("write hosts/<name>/boot.nix by hand"), so
    /// its error is its own message, as a [Failure]
    async fn eval<T: serde::de::DeserializeOwned>(&self, apply: &str) -> eyre::Result<T> {
        json(&self.file, apply).await
    }
}

async fn json<T: serde::de::DeserializeOwned>(file: &Path, apply: &str) -> eyre::Result<T> {
    let output = eval::json(file, apply).await?;
    serde_json::from_slice(&output)
        .wrap_err_with(|| format!("{} answered what coeus does not read", file.display()))
}
