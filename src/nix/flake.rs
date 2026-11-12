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
            .kill_on_drop(true)
            .output()
            .await
            .wrap_err("failed to run nix")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(eyre!("cannot resolve {git_ref}: {}", reason(&stderr)));
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

/// the line of nix's output that says why it failed: git's own `fatal:`
/// when nix fetched through git, else the last `error:` with any text
fn reason(stderr: &str) -> &str {
    let lines = || stderr.lines().map(str::trim);

    lines()
        .find_map(|line| line.strip_prefix("fatal:"))
        .or_else(|| {
            lines()
                .filter_map(|line| line.strip_prefix("error:"))
                .map(str::trim)
                .rfind(|cause| !cause.is_empty())
        })
        .map(str::trim)
        .unwrap_or("nix failed")
}

#[cfg(test)]
mod test {
    #[test]
    fn names_git_or_the_last_error_as_the_reason() {
        let missing = "fatal: couldn't find remote ref refs/heads/deployed\nerror:\n       … while fetching\n\n       error: Failed to fetch git repository 'x'\n";
        assert_eq!(
            super::reason(missing),
            "couldn't find remote ref refs/heads/deployed"
        );

        let nix = "error:\n       … while evaluating\n\n       error: attribute 'altar' missing\n";
        assert_eq!(super::reason(nix), "attribute 'altar' missing");
        assert_eq!(super::reason(""), "nix failed");
    }
}
