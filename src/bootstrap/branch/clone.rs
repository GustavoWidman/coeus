//! making the private clone, and repairing what a stopped run left in it

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, eyre};

use super::git::{Git, check};
use crate::common::model::CommitSha;

/// the clone at `dir`, cloned from the checkout's origin when it is missing
/// or half made. a clone is made under a sibling name and renamed into
/// place, so `dir` is either absent or whole.
pub(super) async fn ensure(dir: &Path, checkout: &Path) -> Result<Git> {
    let origin = Git::at(checkout)
        .run(&["remote", "get-url", "origin"])
        .await
        .wrap_err_with(|| format!("{} has no origin to clone", checkout.display()))?;
    let origin = origin.trim();

    if !is_whole(dir).await? && dir.exists() {
        std::fs::remove_dir_all(dir)
            .wrap_err_with(|| format!("failed to remove the half-made {}", dir.display()))?;
    }
    if !dir.exists() {
        clone(dir, checkout, origin).await?;
    }

    let git = Git::at(dir);
    if git.run(&["remote", "get-url", "origin"]).await?.trim() != origin {
        git.run(&["remote", "set-url", "origin", origin]).await?;
    }
    Ok(git)
}

/// a repository whose top is `dir` itself, not some repository above it
async fn is_whole(dir: &Path) -> Result<bool> {
    if !dir.join(".git").is_dir() {
        return Ok(false);
    }
    let output = Git::at(dir)
        .output(&["rev-parse", "--git-dir"], &[])
        .await?;
    Ok(output.status.success() && output.stdout.trim_ascii() == b".git")
}

async fn clone(dir: &Path, checkout: &Path, origin: &str) -> Result<()> {
    let partial = partial(dir);
    if partial.exists() {
        std::fs::remove_dir_all(&partial)?;
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // borrowing the checkout's objects makes the clone fast; dissociating
    // copies them, so the clone outlives whatever happens to the checkout
    let args = [
        "clone".as_ref(),
        "--quiet".as_ref(),
        "--reference-if-able".as_ref(),
        checkout.as_os_str(),
        "--dissociate".as_ref(),
        "--".as_ref(),
        origin.as_ref(),
        partial.as_os_str(),
    ];
    let output = tokio::process::Command::new("git")
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .wrap_err("failed to run git, is it installed?")?;
    let printable: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
    check(&printable, &output)?;

    std::fs::rename(&partial, dir)
        .wrap_err_with(|| format!("failed to move the clone to {}", dir.display()))
}

fn partial(dir: &Path) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_owned();
    name.push(".partial");
    dir.with_file_name(name)
}

/// what a git killed midway leaves: its `index.lock`, and a rebase or merge
/// it stopped in. the bootstrap's flock proves no git of ours runs here.
pub(super) async fn repair(git: &Git) -> Result<()> {
    let dot_git = git.dir().join(".git");
    let lock = dot_git.join("index.lock");
    if lock.exists() {
        std::fs::remove_file(&lock)?;
    }
    if dot_git.join("rebase-merge").exists() || dot_git.join("rebase-apply").exists() {
        git.run(&["rebase", "--abort"]).await?;
    }
    if dot_git.join("MERGE_HEAD").exists() {
        git.run(&["merge", "--abort"]).await?;
    }
    if dot_git.join("CHERRY_PICK_HEAD").exists() {
        git.run(&["cherry-pick", "--abort"]).await?;
    }
    Ok(())
}

/// origin's default branch, from `refs/remotes/origin/HEAD`
pub(super) async fn base(git: &Git) -> Result<String> {
    let head = [
        "symbolic-ref",
        "--quiet",
        "--short",
        "refs/remotes/origin/HEAD",
    ];
    if !git.test(&head).await? {
        git.run(&["remote", "set-head", "origin", "--auto"]).await?;
    }
    let head = git.run(&head).await?;
    match head.trim().strip_prefix("origin/") {
        Some(base) if !base.is_empty() => Ok(base.to_owned()),
        _ => Err(eyre!(
            "origin's default branch is {:?}, not a branch of origin",
            head.trim()
        )),
    }
}

/// the clone on `branch`. the branch on origin is the record: a local branch
/// that does not contain it is moved to it (another admin rebased or went
/// further), and one that does is ahead with commits not pushed yet.
pub(super) async fn check_out(
    git: &Git,
    branch: &str,
    base: &str,
    remote: Option<CommitSha>,
) -> Result<()> {
    let local = git.resolve(&format!("refs/heads/{branch}")).await?;
    let on_branch = git
        .output(&["symbolic-ref", "--quiet", "--short", "HEAD"], &[])
        .await?;
    let on_branch = String::from_utf8(on_branch.stdout)?.trim() == branch;

    match (local, remote) {
        (Some(_), _) if !on_branch => {
            git.run(&["checkout", "--quiet", branch]).await?;
        }
        (Some(_), _) => {}
        (None, Some(remote)) => {
            let start = remote.to_string();
            git.run(&["checkout", "--quiet", "--no-track", "-b", branch, &start])
                .await?;
        }
        (None, None) => {
            let start = format!("origin/{base}");
            git.run(&["checkout", "--quiet", "--no-track", "-b", branch, &start])
                .await?;
        }
    }

    if let (Some(local), Some(remote)) = (local, remote) {
        let contains = git
            .test(&[
                "merge-base",
                "--is-ancestor",
                &remote.to_string(),
                &local.to_string(),
            ])
            .await?;
        if !contains {
            git.run(&["reset", "--quiet", "--keep", &remote.to_string()])
                .await?;
        }
    }
    Ok(())
}
