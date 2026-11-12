use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
};

use eyre::{Result, WrapErr, bail, eyre};
use log::warn;
use tokio::{fs::File, io::AsyncWriteExt, process::Command};

use crate::common::model::{NarHash, StorePath};

/// working trees uploaded by clients, unpacked under the state directory by
/// NAR hash, where a job's runner builds them as `path:` flakes
#[derive(Clone, Debug)]
pub struct Snapshots {
    root: PathBuf,
}

/// an upload in flight, written to a temporary NAR until complete
pub struct Receiving {
    file: File,
    nar: PathBuf,
    hash: NarHash,
    expected: u64,
    received: u64,
}

impl Snapshots {
    /// larger trees are refused; a configuration repository is megabytes
    pub const LIMIT: u64 = 1 << 30;
    const KEEP: usize = 8;

    pub fn new(state_dir: &Path) -> Self {
        Self {
            root: state_dir.join("sources"),
        }
    }

    pub fn path(&self, hash: &NarHash) -> PathBuf {
        self.root.join(hash.file_name())
    }

    pub fn has(&self, hash: &NarHash) -> bool {
        self.path(hash).is_dir()
    }

    pub async fn receive(&self, hash: NarHash, size: u64) -> Result<Receiving> {
        if size > Self::LIMIT {
            bail!(
                "a snapshot of {size} bytes is over the {} byte limit",
                Self::LIMIT
            );
        }

        tokio::fs::create_dir_all(&self.root).await?;
        let nar = self.staging(&hash, "nar");

        Ok(Receiving {
            file: File::create(&nar).await?,
            nar,
            hash,
            expected: size,
            received: 0,
        })
    }

    /// a source already in the nix store, such as one `nix flake archive`
    /// copied in, unpacked under the sources directory the way an upload
    /// is: to a temporary name, hash-checked, renamed. nix refuses a `path:`
    /// flake that is a symlink, so it is a copy rather than a link to the
    /// store. one already there is kept.
    pub async fn place(&self, source: &StorePath, hash: &NarHash) -> Result<()> {
        if self.has(hash) {
            return Ok(());
        }

        tokio::fs::create_dir_all(&self.root).await?;
        let unpacked = self.staging(hash, "tree");

        let mut dump = Command::new("nix-store")
            .arg("--dump")
            .arg(source.as_str())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .wrap_err("failed to run nix-store")?;
        let nar: Stdio = dump
            .stdout
            .take()
            .ok_or_else(|| eyre!("nix-store --dump has no output"))?
            .try_into()?;

        let restored = restore(&unpacked, nar).await;
        let dumped = dump.wait_with_output().await?;
        if !dumped.status.success() {
            let _ = tokio::fs::remove_dir_all(&unpacked).await;
            bail!(
                "cannot read {source}: {}",
                String::from_utf8_lossy(&dumped.stderr).trim()
            );
        }
        restored?;

        self.settle(&unpacked, hash).await
    }

    /// a temporary name of `hash`'s no other placing shares, in this
    /// process or another, so none unpacks into a tree another is writing
    fn staging(&self, hash: &NarHash, extension: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        self.root.join(format!(
            ".{}.{}.{n}.{extension}",
            hash.file_name(),
            std::process::id()
        ))
    }

    /// checks an unpacked tree hashes to `hash` and puts it in place
    async fn settle(&self, unpacked: &Path, hash: &NarHash) -> Result<()> {
        let hashed = Command::new("nix")
            .args(["--extra-experimental-features", "nix-command"])
            .args(["hash", "path", "--type", "sha256", "--sri"])
            .arg(unpacked)
            .output()
            .await?;
        let actual = String::from_utf8(hashed.stdout)?;

        if actual.trim() != hash.as_str() {
            let _ = tokio::fs::remove_dir_all(unpacked).await;
            return Err(eyre!(
                "the snapshot hashes to {}, not {hash}",
                actual.trim()
            ));
        }

        if let Err(error) = tokio::fs::rename(unpacked, self.path(hash)).await {
            let _ = tokio::fs::remove_dir_all(unpacked).await;
            // another placing of the same tree got there first
            if !self.has(hash) {
                return Err(error.into());
            }
        }
        self.prune()
    }

    /// keeps the newest snapshots only
    fn prune(&self) -> Result<()> {
        let mut snapshots: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        snapshots.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));

        for (_, path) in snapshots.into_iter().skip(Self::KEEP) {
            if let Err(error) = std::fs::remove_dir_all(&path) {
                warn!("failed to prune snapshot {}: {error}", path.display());
            }
        }
        Ok(())
    }
}

impl Receiving {
    /// appends a chunk; true once every byte arrived
    pub async fn write(&mut self, data: &[u8]) -> Result<bool> {
        self.received += data.len() as u64;
        if self.received > self.expected {
            bail!("the upload is longer than announced");
        }

        self.file.write_all(data).await?;
        Ok(self.received == self.expected)
    }

    /// unpacks the NAR, checks it hashes to what the client claimed and puts
    /// it in place
    pub async fn finish(mut self, snapshots: &Snapshots) -> Result<()> {
        self.file.flush().await?;
        let unpacked = self.nar.with_extension("tree");
        let _ = tokio::fs::remove_dir_all(&unpacked).await;

        let nar = std::fs::File::open(&self.nar)?;
        let restored = restore(&unpacked, nar.into()).await;
        let _ = tokio::fs::remove_file(&self.nar).await;
        restored?;

        snapshots.settle(&unpacked, &self.hash).await
    }
}

/// unpacks the NAR read from `nar` at `path`
async fn restore(path: &Path, nar: Stdio) -> Result<()> {
    let restored = Command::new("nix-store")
        .arg("--restore")
        .arg(path)
        .stdin(nar)
        .stderr(Stdio::piped())
        .output()
        .await
        .wrap_err("failed to run nix-store")?;

    if !restored.status.success() {
        let _ = tokio::fs::remove_dir_all(path).await;
        bail!(
            "not a valid NAR: {}",
            String::from_utf8_lossy(&restored.stderr).trim()
        );
    }
    Ok(())
}
