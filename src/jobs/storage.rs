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
const LEDGER_FORMAT: &[u8] = b"coeus-ledger-v2\n";

pub(super) struct JobStorage {
    directory: PathBuf,
    scope: String,
    // The handle lifetime is the ownership lease, including all I/O tasks.
    _lock: File,
    io: std::sync::Mutex<()>,
    high_water: std::sync::atomic::AtomicU64,
}

impl JobStorage {
    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }
    pub async fn open(
        directory: PathBuf,
        scope: String,
    ) -> Result<(Arc<Self>, VecDeque<JobRecord>)> {
        tokio::task::spawn_blocking(move || Self::load(directory, scope, true)).await?
    }

    fn load(
        directory: PathBuf,
        scope: String,
        initialize: bool,
    ) -> Result<(Arc<Self>, VecDeque<JobRecord>)> {
        if scope.len() > 4096 {
            bail!("repository scope exceeds storage limit");
        }
        if !initialize {
            Self::regular_directory(&directory)?;
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
        if initialize {
            // Sync the parent's directory entry too (state_directory's parent must exist).
            File::open(
                directory
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?
            .sync_all()?;
        }
        Self::regular_directory(&directory)?;
        let lock_path = directory.join("ledger.lock");
        if fs::symlink_metadata(&lock_path).is_ok() {
            Self::regular_file(&lock_path)?;
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(initialize)
            .truncate(false);
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
            high_water: std::sync::atomic::AtomicU64::new(0),
        });
        let mut records = Vec::new();
        let mut has_v2_history = false;
        for entry in fs::read_dir(&storage.directory)? {
            let path = entry?.path();
            Self::regular_file(&path)?;
            if path.file_name().is_some_and(|name| name == "id-counter")
                && path.metadata()?.len() != 8
            {
                bail!("invalid job ID counter size");
            }
            if path.file_name().is_some_and(|name| name == "ledger-format")
                && path.metadata()?.len() != LEDGER_FORMAT.len() as u64
            {
                bail!("invalid job ledger format marker size");
            }
            if path.file_name().is_some_and(|name| {
                name == "ledger.lock"
                    || name == "snapshot.pending"
                    || name == "id-counter.pending"
                    || name == "id-counter"
                    || name == "ledger-format"
                    || name == "ledger-format.pending"
            }) {
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
            let snapshot = Snapshot::decode(&bytes)?;
            snapshot.validate(&storage.scope, &path)?;
            has_v2_history |= snapshot.version == 2;
            records.push(snapshot.into_record());
        }
        records.sort_by_key(|record| record.sequence);
        for pair in records.windows(2) {
            if pair[0].sequence == pair[1].sequence {
                bail!("duplicate job admission sequence");
            }
        }
        let mut nonces = std::collections::HashSet::new();
        for record in &records {
            if !nonces.insert(record.request.request_id) {
                bail!("duplicate retained request nonce");
            }
        }
        let marker = storage.directory.join("ledger-format");
        let initialized = match fs::read(&marker) {
            Ok(bytes) if bytes == LEDGER_FORMAT => true,
            Ok(_) => {
                bail!("invalid job ledger format marker");
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        let counter = storage.directory.join("id-counter");
        let saved = match fs::read(&counter) {
            Ok(bytes) => Some(u64::from_be_bytes(
                bytes
                    .try_into()
                    .map_err(|_| eyre::eyre!("invalid job ID counter"))?,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let legacy_max = records.iter().map(|r| r.status.job_id).max().unwrap_or(0);
        match saved {
            Some(saved) if saved < legacy_max => {
                bail!("job ID counter precedes retained history");
            }
            None if initialized
                || has_v2_history
                || storage.directory.join("ledger-format.pending").exists()
                || storage.directory.join("id-counter.pending").exists() =>
            {
                bail!("initialized job ledger is missing its ID counter; reconcile offline");
            }
            _ => {}
        }
        if !initialize {
            eyre::ensure!(
                initialized && saved.is_some(),
                "operator recovery requires an existing initialized v2 ledger"
            );
        }
        // A genuinely new directory or v1-only migration initializes the counter
        // before the marker, and both precede any v2 snapshot or admission. Never
        // reconstruct an initialized v2 counter from retained (possibly pruned) IDs.
        let high_water = saved.unwrap_or(legacy_max);
        if saved.is_none() {
            storage.write_counter(high_water)?;
        } else {
            storage
                .high_water
                .store(high_water, std::sync::atomic::Ordering::Relaxed);
        }
        if !initialized {
            storage.write_atomic("ledger-format", LEDGER_FORMAT)?;
        }
        Ok((storage, records.into()))
    }

    pub(super) fn existing(
        directory: PathBuf,
        scope: String,
    ) -> Result<(Arc<Self>, VecDeque<JobRecord>)> {
        Self::load(directory, scope, false)
    }

    pub fn high_water(&self) -> u64 {
        self.high_water.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// Reserve before snapshot/ACK. Crashes may burn a number, never reuse one.
    pub async fn reserve_id(self: &Arc<Self>, id: u64) -> Result<()> {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = storage
                .io
                .lock()
                .map_err(|_| eyre::eyre!("counter writer poisoned"))?;
            if id <= storage.high_water() {
                bail!("job ID must increase");
            }
            storage.write_counter(id)?;
            Ok(())
        })
        .await?
    }

    // Caller owns the I/O mutex, or the storage has not yet been published by load.
    fn write_counter(&self, id: u64) -> Result<()> {
        self.write_atomic("id-counter", &id.to_be_bytes())?;
        self.high_water
            .store(id, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn write_atomic(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let pending = self.directory.join(format!("{name}.pending"));
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
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(pending, self.directory.join(name))?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }

    pub async fn save(self: &Arc<Self>, record: &JobRecord) -> Result<()> {
        let snapshot = Snapshot::from_record(record, self.scope.clone());
        let storage = self.clone();
        tokio::task::spawn_blocking(move || {
            let result = storage.write(snapshot);
            drop(storage); // release the lease reference before completion is observed
            result
        })
        .await?
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
        #[cfg(test)]
        snapshot_sync_fault::check(&self.directory)?;
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
            if metadata.permissions().mode() & 0o077 != 0
                || metadata.nlink() != 1
                || metadata.uid() != nix::unistd::geteuid().as_raw()
            {
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
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.uid() != nix::unistd::geteuid().as_raw()
                || metadata.permissions().mode() & 0o077 != 0
            {
                bail!("ledger directory must have private permissions (0700)");
            }
        }
        Ok(())
    }
}

// Only snapshot writes: never counter/format writes or production I/O. Unique
// temporary directories isolate parallel tests; dropping a guard disarms it.
#[cfg(test)]
pub(super) mod snapshot_sync_fault {
    use std::{
        collections::HashSet,
        path::{Path, PathBuf},
        sync::Mutex,
    };
    static FAILURES: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    pub(in crate::jobs) struct Failure(PathBuf);
    pub(in crate::jobs) fn fail_next(directory: &Path) -> Failure {
        assert!(
            FAILURES
                .lock()
                .unwrap()
                .get_or_insert_default()
                .insert(directory.into()),
            "snapshot sync fault already armed for directory"
        );
        Failure(directory.into())
    }
    pub(super) fn check(directory: &Path) -> eyre::Result<()> {
        if FAILURES
            .lock()
            .unwrap()
            .as_mut()
            .is_some_and(|paths| paths.remove(directory))
        {
            eyre::bail!("injected post-rename snapshot directory fsync failure");
        }
        Ok(())
    }
    impl Drop for Failure {
        fn drop(&mut self) {
            if let Some(paths) = FAILURES.lock().unwrap().as_mut() {
                paths.remove(&self.0);
            }
        }
    }

    #[test]
    fn fault_is_directory_scoped_one_shot_and_unused_guard_disarms() -> eyre::Result<()> {
        let first = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        let guard = fail_next(first.path());
        check(other.path())?;
        assert!(check(first.path()).is_err());
        check(first.path())?;
        drop(guard);
        let unused = fail_next(first.path());
        drop(unused);
        check(first.path())?;
        Ok(())
    }
}
