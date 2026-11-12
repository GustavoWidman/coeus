//! the pull request through `gh`, found by its head branch, so a run that
//! died after `gh pr create` finds the one it opened. only a pull request
//! from the repository itself counts: anyone can open one from a fork
//! whose branch has the same name.

use std::{fmt, path::Path, process::Stdio};

use eyre::{Result, WrapErr, eyre};
use serde::Deserialize;
use tokio::{io::AsyncWriteExt, process::Command};

use super::{body, git::REDIRECTING};
use crate::common::model::CommitSha;

const FIELDS: &str = "number,url,isDraft,state,body,headRefOid,isCrossRepository";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub draft: bool,
    pub state: PullState,
    pub body: String,
    /// the commit its head branch points at, as the forge saw it last
    pub head: CommitSha,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PullState {
    Open,
    Merged,
    /// closed without merging: left alone, and a new one opened
    Closed,
}

/// `gh` missing, signed out or refusing: only the person can fix it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhError {
    pub command: String,
    pub message: String,
}

impl fmt::Display for GhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gh {} failed: {}", self.command, self.message)
    }
}

impl std::error::Error for GhError {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    number: u64,
    url: String,
    is_draft: bool,
    state: PullState,
    body: String,
    head_ref_oid: CommitSha,
    is_cross_repository: bool,
}

impl From<Wire> for PullRequest {
    fn from(wire: Wire) -> Self {
        Self {
            number: wire.number,
            url: wire.url,
            draft: wire.is_draft,
            state: wire.state,
            body: wire.body,
            head: wire.head_ref_oid,
        }
    }
}

/// `gh pr list --json` output: the pull request of the branch, if any, from
/// the repository's own pull requests
pub(super) fn parse_list(json: &str) -> Result<Option<PullRequest>> {
    let listed: Vec<Wire> = serde_json::from_str(json).wrap_err("unexpected gh pr list output")?;
    Ok(PullRequest::pick(
        listed
            .into_iter()
            .filter(|wire| !wire.is_cross_repository)
            .map(PullRequest::from),
    ))
}

/// `gh pr view --json` output
pub(super) fn parse_view(json: &str) -> Result<PullRequest> {
    let wire: Wire = serde_json::from_str(json).wrap_err("unexpected gh pr view output")?;
    Ok(wire.into())
}

impl PullRequest {
    /// of a branch's pull requests, the newest open or merged one. a merged
    /// one means the work is done; one closed without merging is history,
    /// and a new one is opened beside it, since GitHub does not reopen a
    /// pull request whose branch was force-pushed since
    pub fn pick(listed: impl IntoIterator<Item = Self>) -> Option<Self> {
        listed
            .into_iter()
            .filter(|pull| pull.state != PullState::Closed)
            .max_by_key(|pull| pull.number)
    }

    /// `gh pr list --head <branch> --state all --json …`, as [Self::pick]
    /// chooses among the repository's own
    pub async fn find(clone: &Path, branch: &str) -> Result<Option<Self>> {
        let args = [
            "pr", "list", "--head", branch, "--state", "all", "--json", FIELDS,
        ];
        parse_list(&gh(clone, &args, None).await?)
    }

    /// `gh pr create --draft --base <base> --head <branch> --title … --body …`
    pub async fn create_draft(
        clone: &Path,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<Self> {
        let args = [
            "pr",
            "create",
            "--draft",
            "--base",
            base,
            "--head",
            branch,
            "--title",
            title,
            "--body-file",
            "-",
        ];
        let url = gh(clone, &args, Some(body)).await?;
        let url = url.lines().last().unwrap_or_default().trim();
        Self::view(clone, url).await
    }

    async fn view(clone: &Path, number_or_url: &str) -> Result<Self> {
        let args = ["pr", "view", number_or_url, "--json", FIELDS];
        parse_view(&gh(clone, &args, None).await?)
    }

    /// `gh pr edit --body` with coeus's section replaced, only when it differs
    pub async fn edit_body_section(&mut self, clone: &Path, section: &str) -> Result<()> {
        let body = body::splice(&self.body, section);
        if body == self.body {
            return Ok(());
        }
        let number = self.number.to_string();
        gh(
            clone,
            &["pr", "edit", &number, "--body-file", "-"],
            Some(&body),
        )
        .await?;
        *self = Self::view(clone, &number).await?;
        if body::splice(&self.body, section) != self.body {
            return Err(eyre!(
                "pull request #{number} does not show coeus's section"
            ));
        }
        Ok(())
    }

    /// `gh pr ready`, unless it is ready already
    pub async fn ready(&mut self, clone: &Path) -> Result<()> {
        if !self.draft {
            return Ok(());
        }
        let number = self.number.to_string();
        gh(clone, &["pr", "ready", &number], None).await?;
        *self = Self::view(clone, &number).await?;
        if self.draft {
            return Err(eyre!("pull request #{number} is still a draft"));
        }
        Ok(())
    }
}

/// `gh` in the clone, so it picks the repository from origin; stdout
async fn gh(clone: &Path, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let failed = |message: String| GhError {
        command: args.iter().take(2).copied().collect::<Vec<_>>().join(" "),
        message,
    };
    let mut command = Command::new("gh");
    // gh finds the repository through git, which these would point elsewhere
    for key in REDIRECTING {
        command.env_remove(key);
    }
    let mut child = command
        .args(args)
        .current_dir(clone)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| failed(format!("gh could not start ({error}); is it installed?")))?;

    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(text.as_bytes()).await?;
    }
    let output = child.wait_with_output().await?;
    if !output.status.success() {
        return Err(failed(String::from_utf8_lossy(&output.stderr).trim().to_owned()).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}
