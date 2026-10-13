//! Single-owner, bounded atomic snapshots. No deployment is replayed from disk.
use super::{
    ledger::{JobRecord, MAX_JOBS},
    snapshot::Snapshot,
};
use eyre::{Result, bail};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_SNAPSHOT: usize = 300 * 1024;

pub(super) struct JobStorage {
    directory: PathBuf,
    scope: String,
    // The handle lifetime is the ownership lease, including all I/O tasks.
    _lock: File,
    io: std::sync::Mutex<()>,
}

impl JobStorage {
    pub async fn open(
        directory: PathBuf,
        scope: String,
    ) -> Result<(Arc<Self>, VecDeque<JobRecord>)> {
        tokio::task::spawn_blocking(move || Self::load(directory, scope)).await?
    }

    fn load(directory: PathBuf, scope: String) -> Result<(Arc<Self>, VecDeque<JobRecord>)> {
        if scope.len() > 4096 {
            bail!("repository scope exceeds storage limit");
        }
        if !directory.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new().mode(0o700).create(&directory)?;
            }
            #[cfg(not(unix))]
            fs::create_dir(&directory)?;
        }
        // Sync the parent's directory entry too (state_directory's parent must exist).
        File::open(
            directory
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()?;
        Self::regular_directory(&directory)?;
        let lock_path = directory.join("ledger.lock");
        if fs::symlink_metadata(&lock_path).is_ok() {
            Self::regular_file(&lock_path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
        }
        let lock = options.open(lock_path)?;
        lock.try_lock()
            .map_err(|_| eyre::eyre!("job ledger is already owned by another daemon"))?;
        let storage = Arc::new(Self {
            directory,
            scope,
            _lock: lock,
            io: std::sync::Mutex::new(()),
        });
        let mut records = Vec::new();
        for entry in fs::read_dir(&storage.directory)? {
            let path = entry?.path();
            Self::regular_file(&path)?;
            if path
                .file_name()
                .is_some_and(|name| name == "ledger.lock" || name == "snapshot.pending")
            {
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "job") {
                bail!("unexpected file in job ledger");
            }
            if records.len() == MAX_JOBS {
                bail!("job ledger exceeds retention bound");
            }
            if path.metadata()?.len() > MAX_SNAPSHOT as u64 {
                bail!("job snapshot exceeds size bound");
            }
            let bytes = fs::read(&path)?;
            let (snapshot, remaining): (Snapshot, _) = postcard::take_from_bytes(&bytes)?;
            if !remaining.is_empty() {
                bail!("trailing data in job snapshot");
            }
            snapshot.validate(&storage.scope, &path)?;
            records.push(snapshot.into_record());
        }
        records.sort_by_key(|record| record.sequence);
        for pair in records.windows(2) {
            if pair[0].sequence == pair[1].sequence {
                bail!("duplicate job admission sequence");
            }
        }
        Ok((storage, records.into()))
    }

    pub async fn save(self: &Arc<Self>, record: &JobRecord) -> Result<()> {
        let snapshot = Snapshot::from_record(record, self.scope.clone());
        let storage = self.clone();
        tokio::task::spawn_blocking(move || storage.write(snapshot)).await?
    }

    fn write(&self, snapshot: Snapshot) -> Result<()> {
        let _guard = self
            .io
            .lock()
            .map_err(|_| eyre::eyre!("snapshot writer lock poisoned"))?;
        let bytes = postcard::to_stdvec(&snapshot)?;
        if bytes.len() > MAX_SNAPSHOT {
            bail!("job snapshot exceeds size bound");
        }
        let pending = self.directory.join("snapshot.pending");
        if fs::symlink_metadata(&pending).is_ok() {
            Self::regular_file(&pending)?;
        }
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
        }
        let mut file = options.open(&pending)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(
            &pending,
            self.directory
                .join(format!("{}.job", snapshot.status.job_id)),
        )?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }

    fn regular_file(path: &Path) -> Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() {
            bail!("ledger contains a non-regular file");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.permissions().mode() & 0o077 != 0 || metadata.nlink() != 1 {
                bail!("ledger files must be private and not hard-linked");
            }
        }
        Ok(())
    }

    fn regular_directory(path: &Path) -> Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            bail!("ledger directory must not be a symlink");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                bail!("ledger directory must have private permissions (0700)");
            }
        }
        Ok(())
    }
}
