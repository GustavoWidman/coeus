//! what a new machine adds to the repository, checked

use std::{collections::BTreeMap, fmt};

use eyre::{Report, bail};

use super::wire::{self, FirstSwitch, Messages};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enrollment {
    /// written when absent; one that exists is never edited, so a person
    /// may write it first and a run that crashed left it right
    pub files: BTreeMap<RepoPath, String>,
    /// the machine's own secrets, made only when absent
    pub secrets: BTreeMap<RepoPath, Generator>,
    pub first_switch: FirstSwitch,
    pub messages: Messages,
    pub notes: Vec<String>,
}

impl TryFrom<wire::Enrollment> for Enrollment {
    type Error = Report;

    fn try_from(enrollment: wire::Enrollment) -> eyre::Result<Self> {
        // TODO: every path a RepoPath; every secret path ending in `.age`
        // and every file path not; no path both a file and a secret; each
        // secret's kind into a Generator, refusing a field its kind does not
        // use; a random secret of 0 bytes refused
        todo!()
    }
}

/// a path inside the repository: relative, normalized, no `..`, nothing
/// under `.git`. the only way a path from the contract reaches the disk.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPath(String);

impl RepoPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_secret(&self) -> bool {
        self.0.ends_with(".age")
    }
}

impl TryFrom<String> for RepoPath {
    type Error = Report;

    fn try_from(path: String) -> eyre::Result<Self> {
        if path.is_empty() || path.contains('\0') || path.contains('\\') {
            bail!("not a repository path: {path:?}");
        }
        if path.starts_with('/') {
            bail!("a repository path is relative: {path:?}");
        }
        // `Path::components` would hide `a//b` and `a/./b`; split instead so
        // a path has exactly one spelling
        for part in path.split('/') {
            let wrong = match part {
                "" | "." => "is not normalized",
                ".." => "leaves the repository",
                ".git" => "touches .git",
                _ => continue,
            };
            bail!("the repository path {path:?} {wrong}");
        }
        Ok(Self(path))
    }
}

impl std::str::FromStr for RepoPath {
    type Err = Report;

    fn from_str(path: &str) -> eyre::Result<Self> {
        Self::try_from(path.to_owned())
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// how coeus makes a secret: a closed set it implements in memory
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Generator {
    /// typed twice by the person, stored as a sha512-crypt hash (`$6$`),
    /// which NixOS's `hashedPasswordFile` takes
    Password { prompt: Option<String> },
    /// a new OpenSSH ed25519 private key
    SshEd25519 { comment: Option<String> },
    /// random bytes, stored as lowercase hex
    Random { bytes: usize },
    /// a value the person pastes, such as a token
    Text { prompt: Option<String> },
}
