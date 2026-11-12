use std::fmt;

use serde::{Deserialize, Serialize};

use super::{CommitSha, NarHash};

/// what a job deploys. always pinned: a branch name never reaches a daemon,
/// so every machine of one order builds exactly the same thing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    /// a commit of the daemon's configured flake repository, with the ref it
    /// was resolved from when there was one
    Git {
        rev: CommitSha,
        git_ref: Option<String>,
    },
    /// a working tree uploaded by the client, addressed by its NAR hash
    Snapshot { nar_hash: NarHash, label: String },
    /// an existing generation of the system profile (a rollback)
    Generation(u32),
}

impl Source {
    /// the ref this source follows, if it came from one
    pub fn git_ref(&self) -> Option<&str> {
        match self {
            Self::Git { git_ref, .. } => git_ref.as_deref(),
            _ => None,
        }
    }

    /// whether the job has to fetch, evaluate and build before activating
    pub const fn builds(&self) -> bool {
        !matches!(self, Self::Generation(_))
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git {
                rev,
                git_ref: Some(git_ref),
            } => write!(f, "{git_ref}@{}", rev.short()),
            Self::Git { rev, git_ref: None } => f.write_str(&rev.short()),
            Self::Snapshot { label, .. } => write!(f, "snapshot of {label}"),
            Self::Generation(generation) => write!(f, "generation {generation}"),
        }
    }
}

/// whether `git_ref` is a plain branch or tag name, safe to put in a flake
/// url: letters, digits, `.`, `_`, `-` and `/`, without `..`, `//` or a
/// leading `-`, `/` or `.`
pub fn valid_ref(git_ref: &str) -> bool {
    !git_ref.is_empty()
        && git_ref.len() <= 255
        && !git_ref.starts_with(['-', '/', '.'])
        && !git_ref.ends_with(['/', '.'])
        && !git_ref.contains("..")
        && !git_ref.contains("//")
        && git_ref
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
}

/// a ref resolved to the commit it pointed at
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pinned {
    pub git_ref: String,
    pub rev: CommitSha,
    /// commit time, from the flake's `lastModified`
    pub committed_at: super::Timestamp,
}

impl From<Pinned> for Source {
    fn from(pinned: Pinned) -> Self {
        Self::Git {
            rev: pinned.rev,
            git_ref: Some(pinned.git_ref),
        }
    }
}
