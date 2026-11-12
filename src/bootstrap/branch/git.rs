//! `git` in the private clone, as the person: their identity, signing and
//! credentials come from their own git config

use std::{
    path::{Path, PathBuf},
    process::Output,
};

use eyre::{Result, WrapErr, bail};
use tokio::process::Command;

use crate::common::model::CommitSha;

pub(super) struct Git {
    dir: PathBuf,
}

impl Git {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// stdout of a command that must succeed
    pub async fn run<S: AsRef<str>>(&self, args: &[S]) -> Result<String> {
        let output = self.output(args, &[]).await?;
        check(args, &output)?;
        Ok(String::from_utf8(output.stdout)?)
    }

    /// whether a command succeeded, for questions git answers by exit code
    pub async fn test<S: AsRef<str>>(&self, args: &[S]) -> Result<bool> {
        Ok(self.output(args, &[]).await?.status.success())
    }

    pub async fn output<S: AsRef<str>>(&self, args: &[S], env: &[(&str, &str)]) -> Result<Output> {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.dir)
            .args(args.iter().map(AsRef::as_ref))
            // the run owns the terminal; a credential prompt would draw over it
            .env("GIT_TERMINAL_PROMPT", "0");
        for (key, value) in env {
            command.env(key, value);
        }
        command
            .output()
            .await
            .wrap_err("failed to run git, is it installed?")
    }

    /// a revision's commit, or `None` when it does not resolve
    pub async fn resolve(&self, revision: &str) -> Result<Option<CommitSha>> {
        let commit = format!("{revision}^{{commit}}");
        let output = self
            .output(&["rev-parse", "--verify", "--quiet", &commit], &[])
            .await?;
        if !output.status.success() {
            return Ok(None);
        }
        Ok(Some(String::from_utf8(output.stdout)?.trim().parse()?))
    }

    /// NUL-separated paths, as `-z` prints them
    pub async fn paths<S: AsRef<str>>(&self, args: &[S]) -> Result<Vec<String>> {
        Ok(split_nul(&self.run(args).await?))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

pub(super) fn check<S: AsRef<str>>(args: &[S], output: &Output) -> Result<()> {
    if !output.status.success() {
        let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(super) fn split_nul(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// the paths `git status --porcelain=v1 -z` lists; a rename or copy names
/// its source in a second entry, which is not a change of its own
pub(super) fn status_paths(text: &str) -> Vec<String> {
    let mut entries = text.split('\0').filter(|entry| !entry.is_empty());
    let mut paths = Vec::new();
    while let Some(entry) = entries.next() {
        let Some((status, path)) = entry.split_at_checked(3) else {
            continue;
        };
        if status.starts_with(['R', 'C']) {
            entries.next();
        }
        paths.push(path.to_owned());
    }
    paths
}
