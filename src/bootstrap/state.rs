//! what an admin keeps of one machine's bootstrap:
//!
//! ```text
//! $XDG_STATE_HOME/coeus/bootstrap/<host>/   (~/.local/state/… without it)
//!   bootstrap.toml       Saved: how to reach it, the repository, answers,
//!                        the stock machine's facts
//!   known_hosts          the one ed25519 host key the person trusted
//!   session_key[.pub]    the key root's authorized_keys holds until cleanup
//!   lock                 flock held by the run in progress
//!   ssh-<user>           the control masters' sockets
//!   repo/                the private clone, on coeus/bootstrap/<host>
//! ```
//!
//! nothing here records progress: a run decides what is left from the
//! branch, the secrets, the machine and the forge.

use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{Read, Seek, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};

use crate::{
    bootstrap::{contract::Answers, remote::Facts},
    common::model::{Hostname, Timestamp},
};

pub struct StateDir {
    host: Hostname,
    root: PathBuf,
}

/// the inputs a run cannot observe again, written through on every change
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// how ssh reaches the stock machine, as the person typed it: an alias
    /// of their ssh config works
    pub destination: Option<String>,
    /// the person's checkout of the configuration repository, which the
    /// private clone borrows objects and its origin from
    pub repo: Option<PathBuf>,
    /// the branch of origin the enrollment goes onto; origin's default
    /// branch when unset
    pub base: Option<String>,
    /// the contract's questions answered so far, by id
    pub answers: Answers,
    /// the machine as its stock system showed it before it was enrolled,
    /// which the switch replaces for good: the contract's input keeps
    /// saying the same about it once ssh is gone
    pub facts: Option<Facts>,
    /// display only: no decision reads it
    pub last_run: Option<LastRun>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastRun {
    pub started_at: Timestamp,
    /// `None` when the run never got to say: the admin died, or kill -9
    pub ended: Option<Ended>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ended {
    pub at: Timestamp,
    /// `detached while building`, `in the fleet`, …
    pub how: String,
}

/// held for the life of a run; the kernel drops it with the process
pub struct Lock {
    _file: File,
}

impl StateDir {
    /// under `$XDG_STATE_HOME`, else `~/.local/state`
    pub fn of(host: &Hostname) -> Result<Self> {
        Ok(Self::under(&state_home()?, host))
    }

    /// `<state_home>/coeus/bootstrap/<host>`
    pub fn under(state_home: &Path, host: &Hostname) -> Self {
        Self {
            host: host.clone(),
            root: Self::all_under(state_home).join(host.as_str()),
        }
    }

    /// where every machine's bootstrap state is
    pub fn all_under(state_home: &Path) -> PathBuf {
        state_home.join("coeus").join("bootstrap")
    }

    /// every machine with a bootstrap in progress, for `coeus bootstrap`
    pub fn all() -> Result<Vec<(Hostname, Saved)>> {
        Self::all_in(&state_home()?)
    }

    /// [StateDir::all] under `state_home`, by name
    pub fn all_in(state_home: &Path) -> Result<Vec<(Hostname, Saved)>> {
        let root = Self::all_under(state_home);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).wrap_err_with(|| format!("{}", root.display())),
        };
        let mut all = Vec::new();
        for entry in entries {
            let entry = entry?;
            let Some(host) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<Hostname>().ok())
            else {
                continue;
            };
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let saved = Self::under(state_home, &host).load()?;
            all.push((host, saved));
        }
        all.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(all)
    }

    pub fn host(&self) -> &Hostname {
        &self.host
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn saved(&self) -> PathBuf {
        self.root.join("bootstrap.toml")
    }

    pub fn known_hosts(&self) -> PathBuf {
        self.root.join("known_hosts")
    }

    /// the private half; the public half is beside it with `.pub`
    pub fn session_key(&self) -> PathBuf {
        self.root.join("session_key")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.root.join("lock")
    }

    /// an ssh `ControlPath`: one master per remote user, since the login
    /// user's master and root's session key master live side by side
    pub fn control_path(&self) -> PathBuf {
        self.root.join("ssh-%r")
    }

    pub fn clone_dir(&self) -> PathBuf {
        self.root.join("repo")
    }

    /// fails naming the pid when another run of this machine holds it. a
    /// lock just let go may still be held for a moment by a process the
    /// last run started, so it is asked for again for a little while.
    pub async fn lock(&self) -> Result<Lock> {
        const GRACE: Duration = Duration::from_secs(2);
        const AGAIN: Duration = Duration::from_millis(50);
        let started = std::time::Instant::now();
        loop {
            match self.try_lock()? {
                Ok(lock) => return Ok(lock),
                Err(_) if started.elapsed() < GRACE => tokio::time::sleep(AGAIN).await,
                Err(pid) => {
                    return Err(eyre!(
                        "another `coeus bootstrap {}` is running (pid {pid})",
                        self.host
                    ));
                }
            }
        }
    }

    /// whether a run of this machine holds the lock now
    pub fn locked(&self) -> Result<bool> {
        Ok(self.root.exists() && self.try_lock()?.is_err())
    }

    /// the lock, or the pid of the run that holds it
    fn try_lock(&self) -> Result<Result<Lock, String>> {
        std::fs::create_dir_all(&self.root)
            .wrap_err_with(|| format!("could not make {}", self.root.display()))?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.lock_file())?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let mut pid = String::new();
                file.read_to_string(&mut pid)?;
                return Ok(Err(pid.trim().to_owned()));
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        file.set_len(0)?;
        file.rewind()?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Ok(Lock { _file: file }))
    }

    /// the saved inputs, or the defaults on a first run
    pub fn load(&self) -> Result<Saved> {
        let path = self.saved();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).wrap_err_with(|| format!("in {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Saved::default()),
            Err(error) => Err(error).wrap_err_with(|| format!("could not read {}", path.display())),
        }
    }

    /// through a temporary sibling, fsync and a rename, 0600
    pub fn save(&self, saved: &Saved) -> Result<()> {
        let text = toml::to_string(saved).wrap_err("could not write the saved inputs")?;
        crate::bootstrap::remote::write_private(&self.saved(), text.as_bytes())
    }

    /// the session key's two halves gone from here, once root's
    /// `authorized_keys` no longer holds it; the private half first, since a
    /// public half alone is no key
    pub fn drop_session_key(&self) -> Result<()> {
        let private = self.session_key();
        let mut public = private.as_os_str().to_owned();
        public.push(".pub");
        for path in [private, PathBuf::from(public)] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// removes this directory: the saved inputs, the pinned key, the session
    /// key and the clone. the branch and the pull request stay, and a later
    /// run picks them up again. `lock` is held until everything else is
    /// gone, so no run starts on a clone being removed; a run that starts
    /// once the lock file is gone makes a new one and starts afresh.
    pub async fn forget(self, lock: Lock) -> Result<()> {
        let root = self.root.clone();
        let lock_file = self.lock_file();
        tokio::task::spawn_blocking(move || {
            let removed = remove_all_but(&root, &lock_file)
                .and_then(|()| gone(std::fs::remove_file(&lock_file)))
                .wrap_err_with(|| format!("could not remove {}", root.display()));
            // a run that started meanwhile keeps the directory
            let emptied = match std::fs::remove_dir(&root) {
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
                result => gone(result),
            };
            drop(lock);
            removed?;
            emptied.wrap_err_with(|| format!("could not remove {}", root.display()))
        })
        .await?
    }
}

/// everything in `dir` but `kept`
fn remove_all_but(dir: &Path, kept: &Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        entries => entries?,
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path == kept {
            continue;
        }
        gone(match entry.file_type()?.is_dir() {
            true => std::fs::remove_dir_all(&path),
            false => std::fs::remove_file(&path),
        })?;
    }
    Ok(())
}

/// a removal, where something already gone counts as removed
fn gone(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// `$XDG_STATE_HOME` when it is an absolute path, as the spec asks, else
/// `$HOME/.local/state`
fn state_home() -> Result<PathBuf> {
    match std::env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => Ok(path),
        _ => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".local").join("state"))
            .ok_or_else(|| eyre!("neither XDG_STATE_HOME nor HOME is set")),
    }
}
