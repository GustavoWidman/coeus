use std::borrow::Cow;

use eyre::{Result, WrapErr, eyre};
use serde::Deserialize;
use tokio::process::Command;

use super::strip_ansi;
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

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = strip_ansi(&stderr);
        if !output.status.success() || fell_back(&stderr) {
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

/// whether nix could not reach the repository and went on with what it
/// fetched before. it only warns and exits as if the fetch worked, so a ref
/// would seem to point where it did when last fetched.
fn fell_back(stderr: &str) -> bool {
    const WARNINGS: [&str; 2] = [
        // the git fetcher
        "continuing with the most recent version",
        // the tarball fetchers, `github:` among them
        "; using cached version",
    ];

    stderr
        .lines()
        .filter(|line| line.trim_start().starts_with("warning:"))
        .any(|line| WARNINGS.iter().any(|warning| line.contains(warning)))
}

/// the line of nix's output that says why it failed: git's own `fatal:`
/// when nix fetched through git, else the last `error:` with any text
fn reason(stderr: &str) -> Cow<'_, str> {
    let mut lines = stderr.lines().map(str::trim);

    if let Some(fatal) = lines.find_map(|line| line.strip_prefix("fatal:")) {
        let fatal = fatal.trim();
        // git breaks the cause onto the next line, as in
        // `unable to connect to host:`
        return match (fatal.ends_with(':'), lines.next()) {
            (true, Some(cause)) if !cause.is_empty() => format!("{fatal} {cause}").into(),
            _ => fatal.into(),
        };
    }

    stderr
        .lines()
        .filter_map(|line| line.trim().strip_prefix("error:"))
        .map(str::trim)
        .rfind(|cause| !cause.is_empty())
        .unwrap_or("nix failed")
        .into()
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

    #[test]
    fn a_fetch_that_fell_back_to_the_cache_is_a_failure() {
        let git = "\u{1b}[31;1merror:\u{1b}[0m executing \u{1b}[35;1m\"git\"\u{1b}[0m: \u{1b}[35;1mConnection refused\u{1b}[0m\nerror: Failed to fetch git repository 'git://10.0.2.2:9518/tree'\nwarning: could not update local clone of Git repository 'git://10.0.2.2:9518/tree'; continuing with the most recent version\n";
        let git = super::strip_ansi(git);
        assert!(super::fell_back(&git));
        assert_eq!(
            super::reason(&git),
            "Failed to fetch git repository 'git://10.0.2.2:9518/tree'"
        );

        let tarball = "warning: unable to download 'https://api.github.com/repos/o/nix/commits/deployed': Couldn't resolve host name (6); using cached version\n";
        assert!(super::fell_back(tarball));

        let refused = "fatal: unable to connect to 10.0.2.2:\n10.0.2.2[0: 10.0.2.2]: errno=Connection refused\n\nerror: Failed to fetch git repository 'git://10.0.2.2:9518/tree'\n";
        assert_eq!(
            super::reason(refused),
            "unable to connect to 10.0.2.2: 10.0.2.2[0: 10.0.2.2]: errno=Connection refused"
        );

        assert!(!super::fell_back(""));
        assert!(!super::fell_back(
            "warning: Git tree '/home/me/nix' is dirty\n"
        ));
    }
}
