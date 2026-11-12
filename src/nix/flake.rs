use eyre::{Result, WrapErr, eyre};
use serde::Deserialize;
use tokio::process::Command;

use crate::common::model::{CommitSha, Pinned, Timestamp};

/// a flake repository as a daemon's configuration names it, e.g.
/// `git+https://github.com/owner/nix`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlakeRepo(String);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    revision: String,
    last_modified: i64,
}

impl FlakeRepo {
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// the repository at `git_ref`
    pub fn at_ref(&self, git_ref: &str) -> String {
        format!("{}{}ref={git_ref}", self.0, self.separator())
    }

    /// the repository at exactly `rev`, keeping the ref it came from for
    /// display and for the fetcher's refspec
    pub fn at_rev(&self, rev: &CommitSha, git_ref: Option<&str>) -> String {
        match git_ref {
            Some(git_ref) => format!("{}&rev={rev}", self.at_ref(git_ref)),
            None => format!("{}{}rev={rev}", self.0, self.separator()),
        }
    }

    /// pins `git_ref` to the commit it points at right now
    pub async fn resolve(&self, git_ref: &str) -> Result<Pinned> {
        let output = Command::new("nix")
            .args(["flake", "metadata", "--json", "--refresh"])
            .args(["--extra-experimental-features", "nix-command flakes"])
            .arg(self.at_ref(git_ref))
            .env("NIX_REMOTE", "daemon")
            .output()
            .await
            .wrap_err("failed to run nix")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr
                .lines()
                .find(|line| line.contains("error"))
                .unwrap_or(&stderr);
            return Err(eyre!("cannot resolve {git_ref}: {}", reason.trim()));
        }

        let metadata: Metadata = serde_json::from_slice(&output.stdout)?;
        Ok(Pinned {
            git_ref: git_ref.to_owned(),
            rev: metadata.revision.parse()?,
            committed_at: Timestamp(metadata.last_modified * 1000),
        })
    }

    fn separator(&self) -> char {
        match self.0.contains('?') {
            true => '&',
            false => '?',
        }
    }
}
