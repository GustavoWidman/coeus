use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use eyre::{Result, bail, eyre};
use log::warn;
use tokio::{fs::File, io::AsyncWriteExt, process::Command};

use crate::common::model::NarHash;

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
    target: PathBuf,
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
        let nar = self.root.join(format!(".{}.nar", hash.file_name()));

        Ok(Receiving {
            file: File::create(&nar).await?,
            target: self.path(&hash),
            nar,
            hash,
            expected: size,
            received: 0,
        })
    }

    /// keeps the newest snapshots only
    fn prune(&self) -> Result<()> {
        let mut snapshots: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        snapshots.sort_by(|a, b| b.0.cmp(&a.0));

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

        let restored = Command::new("nix-store")
            .arg("--restore")
            .arg(&unpacked)
            .stdin(std::fs::File::open(&self.nar)?)
            .stderr(Stdio::piped())
            .output()
            .await?;
        let _ = tokio::fs::remove_file(&self.nar).await;

        if !restored.status.success() {
            bail!(
                "not a valid NAR: {}",
                String::from_utf8_lossy(&restored.stderr).trim()
            );
        }

        let hashed = Command::new("nix")
            .args(["--extra-experimental-features", "nix-command"])
            .args(["hash", "path", "--type", "sha256", "--sri"])
            .arg(&unpacked)
            .output()
            .await?;
        let actual = String::from_utf8(hashed.stdout)?;

        if actual.trim() != self.hash.as_str() {
            let _ = tokio::fs::remove_dir_all(&unpacked).await;
            return Err(eyre!(
                "the snapshot hashes to {}, not {}",
                actual.trim(),
                self.hash
            ));
        }

        tokio::fs::rename(&unpacked, &self.target).await?;
        snapshots.prune()
    }
}
