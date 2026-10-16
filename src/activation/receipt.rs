//! Private, single-writer, fsync-before-effects receipts. Uncertainty never replays.
use super::ServiceIdentity;
use eyre::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationState {
    Ready,
    Activating,
    Verified,
    Uncertain,
    Cancelled,
    OperatorResolved,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub format: u8,
    pub job_id: u64,
    pub closure: PathBuf,
    pub root: PathBuf,
    pub expected: ServiceIdentity,
    pub previous: Option<ServiceIdentity>,
    pub state: ActivationState,
    pub manual: bool,
    pub previous_process_id: Option<u32>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_resolution: Option<super::operator_recovery::OperatorResolution>,
}

pub fn directory_for_ledger(ledger: &Path) -> Result<PathBuf> {
    let name = ledger
        .file_name()
        .ok_or_else(|| eyre::eyre!("ledger needs a file name"))?;
    let mut name = name.to_os_string();
    name.push(".activation");
    Ok(ledger.with_file_name(name))
}

#[derive(Clone)]
pub struct ReceiptStore {
    directory: PathBuf,
    _lease: std::sync::Arc<File>,
    io: std::sync::Arc<std::sync::Mutex<()>>,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl ReceiptStore {
    pub fn open(directory: &Path) -> Result<Self> {
        Self::acquire(directory, true)
    }
    pub(crate) fn existing(directory: &Path) -> Result<Self> {
        Self::acquire(directory, false)
    }
    fn acquire(directory: &Path, create: bool) -> Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        if !create {
            private(directory, true)?;
        }
        if !directory.exists() {
            fs::DirBuilder::new().mode(0o700).create(directory)?;
        }
        if create {
            File::open(directory.parent().unwrap_or(Path::new(".")))?.sync_all()?;
        }
        private(directory, true)?;
        let lock_path = directory.join("activation.lock");
        if lock_path.exists() {
            private(&lock_path, false)?;
        }
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(lock_path)?;
        lease
            .try_lock()
            .map_err(|_| eyre::eyre!("activation is already owned by another process"))?;
        Ok(Self {
            directory: directory.to_path_buf(),
            _lease: std::sync::Arc::new(lease),
            io: std::sync::Arc::default(),
            active: std::sync::Arc::default(),
        })
    }
    pub fn root(&self, id: u64) -> PathBuf {
        self.directory.join(format!("{id}.root"))
    }
    pub fn load(&self, id: u64) -> Result<Receipt> {
        read(&self.directory, id)
    }
    pub async fn open_async(directory: PathBuf) -> Result<Self> {
        tokio::task::spawn_blocking(move || Self::open(&directory)).await?
    }
    pub async fn load_async(&self, id: u64) -> Result<Receipt> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let result = store.load(id);
            drop(store);
            result
        })
        .await?
    }
    pub async fn save_async(&self, receipt: &Receipt) -> Result<()> {
        let store = self.clone();
        let receipt = receipt.clone();
        tokio::task::spawn_blocking(move || {
            let result = store.save(&receipt);
            drop(store);
            result
        })
        .await?
    }
    pub(crate) fn operation(&self) -> Result<Operation> {
        use std::sync::atomic::Ordering;
        ensure!(
            !self.active.swap(true, Ordering::SeqCst),
            "another activation/reconciliation owns this store"
        );
        Ok(Operation(self.active.clone()))
    }
    pub fn save(&self, receipt: &Receipt) -> Result<()> {
        let _writer = self
            .io
            .lock()
            .map_err(|_| eyre::eyre!("receipt writer poisoned"))?;
        use std::os::unix::fs::OpenOptionsExt;
        ensure!(
            receipt.format == 1 && receipt.job_id != 0,
            "invalid receipt"
        );
        super::operator_recovery::validate_audit(receipt)?;
        let pending = self.directory.join("receipt.pending");
        if pending.exists() {
            private(&pending, false)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&pending)?;
        file.write_all(toml::to_string(receipt)?.as_bytes())?;
        file.sync_all()?;
        fs::rename(
            pending,
            self.directory.join(format!("{}.toml", receipt.job_id)),
        )?;
        #[cfg(test)]
        sync_fault::check(&self.directory)?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}
pub fn read(directory: &Path, id: u64) -> Result<Receipt> {
    private(directory, true)?;
    let path = directory.join(format!("{id}.toml"));
    private(&path, false)?;
    ensure!(
        path.metadata()?.len() < 1024 * 1024,
        "receipt exceeds bound"
    );
    let receipt: Receipt = toml::from_str(&fs::read_to_string(path)?)?;
    ensure!(
        receipt.format == 1 && receipt.job_id == id,
        "incompatible receipt"
    );
    receipt.expected.validate()?;
    super::operator_recovery::validate_audit(&receipt)?;
    ensure!(
        receipt.root == directory.join(format!("{id}.root")),
        "receipt root not owned by this store"
    );
    Ok(receipt)
}
fn private(path: &Path, directory: bool) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file() && metadata.nlink() == 1
        },
        "activation state must not be symlinked/hardlinked"
    );
    ensure!(
        metadata.permissions().mode() & 0o077 == 0
            && metadata.uid() == nix::unistd::geteuid().as_raw(),
        "activation state must be private and owned by the executing user"
    );
    Ok(())
}

pub(crate) struct Operation(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for Operation {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

// Per-directory, one-shot fault at the real post-rename/pre-fsync boundary.
// Tests never change permissions, global filesystem behavior, or production I/O.
#[cfg(test)]
pub(crate) mod sync_fault {
    use std::{
        collections::HashSet,
        path::{Path, PathBuf},
        sync::Mutex,
    };
    static FAILURES: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    pub(crate) struct Failure(PathBuf);
    pub(crate) fn fail_next(directory: &Path) -> Failure {
        FAILURES
            .lock()
            .unwrap()
            .get_or_insert_default()
            .insert(directory.into());
        Failure(directory.into())
    }
    pub(super) fn check(directory: &Path) -> eyre::Result<()> {
        if FAILURES
            .lock()
            .unwrap()
            .as_mut()
            .is_some_and(|paths| paths.remove(directory))
        {
            eyre::bail!("injected post-rename receipt directory fsync failure");
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
}
