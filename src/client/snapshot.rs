use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use eyre::{Result, WrapErr, bail};
use tokio::process::Command;

use crate::common::model::{NarHash, Source};

/// a working tree as git sees it, uncommitted changes and untracked files
/// included, serialized as a NAR. deploying one needs no push: the daemon
/// builds exactly these files.
pub struct Snapshot {
    pub nar_hash: NarHash,
    pub nar: Vec<u8>,
    /// where it came from, e.g. `laptop-mac:~/GitHub/nix (main, dirty)`
    pub label: String,
}

impl Snapshot {
    pub async fn capture(dir: &Path) -> Result<Self> {
        let root = PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"]).await?.trim());
        let stage = Self::stage(&root).await?;

        let nar = nix(&["nar", "pack"], stage.path()).await?;
        let hash = nix(&["hash", "path", "--type", "sha256", "--sri"], stage.path()).await?;
        let nar_hash = NarHash::try_from(String::from_utf8(hash)?.trim().to_owned())?;

        Ok(Self {
            nar_hash,
            nar,
            label: Self::label(&root).await,
        })
    }

    /// the files a snapshot of `dir` holds, copied into a temporary
    /// directory, whose NAR hash is the snapshot's
    pub async fn stage(dir: &Path) -> Result<tempfile::TempDir> {
        let root = PathBuf::from(git(dir, &["rev-parse", "--show-toplevel"]).await?.trim());
        let stage = tempfile::Builder::new()
            .prefix("coeus-snapshot")
            .tempdir()?;
        for file in Self::files(&root).await? {
            copy(&root.join(&file), &stage.path().join(&file))?;
        }
        Ok(stage)
    }

    pub fn source(&self) -> Source {
        Source::Snapshot {
            nar_hash: self.nar_hash.clone(),
            label: self.label.clone(),
        }
    }

    /// tracked files that still exist, plus untracked ones git does not ignore
    async fn files(root: &Path) -> Result<BTreeSet<PathBuf>> {
        let tracked = git(
            root,
            &["ls-files", "--cached", "--recurse-submodules", "-z"],
        )
        .await?;
        let untracked = git(root, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;

        Ok(tracked
            .split('\0')
            .chain(untracked.split('\0'))
            .filter(|file| !file.is_empty())
            .map(PathBuf::from)
            .filter(|file| root.join(file).symlink_metadata().is_ok())
            .collect())
    }

    async fn label(root: &Path) -> String {
        let host = rustix::system::uname()
            .nodename()
            .to_string_lossy()
            .split('.')
            .next()
            .unwrap_or_default()
            .to_owned();
        let home = std::env::var("HOME").unwrap_or_default();
        let path = root.display().to_string();
        let path = match path.strip_prefix(&home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => path,
        };

        let branch = git(root, &["branch", "--show-current"])
            .await
            .unwrap_or_default();
        let dirty = !git(root, &["status", "--porcelain"])
            .await
            .unwrap_or_default()
            .is_empty();

        match (branch.trim(), dirty) {
            ("", true) => format!("{host}:{path} (dirty)"),
            ("", false) => format!("{host}:{path}"),
            (branch, true) => format!("{host}:{path} ({branch}, dirty)"),
            (branch, false) => format!("{host}:{path} ({branch})"),
        }
    }
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let metadata = from.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
    } else {
        std::fs::copy(from, to).wrap_err_with(|| format!("failed to copy {}", from.display()))?;
    }
    Ok(())
}

async fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}

async fn nix(args: &[&str], path: &Path) -> Result<Vec<u8>> {
    let output = Command::new("nix")
        .args(["--extra-experimental-features", "nix-command"])
        .args(args)
        .arg(path)
        .output()
        .await?;
    if !output.status.success() {
        bail!(
            "nix {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}
