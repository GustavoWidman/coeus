//! what an admin keeps of one machine's bootstrap:
//!
//! ```text
//! $XDG_STATE_HOME/coeus/bootstrap/<host>/   (~/.local/state/… without it)
//!   bootstrap.toml       Saved: how to reach it, the repository, answers
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
    fs::File,
    path::{Path, PathBuf},
};

use eyre::{Result, eyre};
use serde::{Deserialize, Serialize};

use crate::{
    bootstrap::contract::Answers,
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
    /// the contract's questions answered so far, by id
    pub answers: Answers,
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
pub struct Lock(File);

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
        todo!()
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

    /// fails naming the pid when another run of this machine holds it
    pub fn lock(&self) -> Result<Lock> {
        todo!()
    }

    /// the saved inputs, or the defaults on a first run
    pub fn load(&self) -> Result<Saved> {
        todo!()
    }

    /// through a temporary sibling, fsync and a rename, 0600
    pub fn save(&self, saved: &Saved) -> Result<()> {
        todo!()
    }

    /// removes this directory: the saved inputs, the pinned key, the session
    /// key and the clone. the branch and the pull request stay, and a later
    /// run picks them up again
    pub async fn forget(self) -> Result<()> {
        todo!()
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
