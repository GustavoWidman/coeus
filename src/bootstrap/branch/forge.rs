//! the pull request through `gh`, found by its head branch, so a run that
//! died after `gh pr create` finds the one it opened

use std::{fmt, path::Path, process::Stdio};

use eyre::{Result, WrapErr, eyre};
use serde::Deserialize;
use tokio::{io::AsyncWriteExt, process::Command};

use super::{body, git::REDIRECTING};

const FIELDS: &str = "number,url,isDraft,state,body";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub draft: bool,
    pub state: PullState,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PullState {
    Open,
    Merged,
    /// closed without merging: reopened only when the person says so
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
}

impl From<Wire> for PullRequest {
    fn from(wire: Wire) -> Self {
        Self {
            number: wire.number,
            url: wire.url,
            draft: wire.is_draft,
            state: wire.state,
            body: wire.body,
        }
    }
}

/// `gh pr list --json` output: the newest pull request, if any
pub(super) fn parse_list(json: &str) -> Result<Option<PullRequest>> {
    let listed: Vec<Wire> = serde_json::from_str(json).wrap_err("unexpected gh pr list output")?;
    Ok(listed
        .into_iter()
        .max_by_key(|wire| wire.number)
        .map(PullRequest::from))
}

/// `gh pr view --json` output
pub(super) fn parse_view(json: &str) -> Result<PullRequest> {
    let wire: Wire = serde_json::from_str(json).wrap_err("unexpected gh pr view output")?;
    Ok(wire.into())
}

impl PullRequest {
    /// `gh pr list --head <branch> --state all --json …`, the newest
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
