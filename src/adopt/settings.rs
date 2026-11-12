//! the few daemon settings a job needs, read from the machine's own
//! configuration in the snapshot rather than from flags, so the first job is
//! guarded exactly as coeusd will guard every later one

use std::{net::SocketAddr, path::PathBuf};

use crate::{
    common::model::{Hostname, StorePath},
    job::Guard,
};

pub struct Settings {
    pub flake: String,
    pub state_dir: PathBuf,
    pub listen: SocketAddr,
    pub ssh_key_file: Option<PathBuf>,
    pub guard: Option<Guard>,
    /// darwin: where the runner launcher goes, installed here once
    pub launcher: Option<PathBuf>,
}

impl Settings {
    /// `nix eval --json path:<source>#<configurations>.<host>.config.services.coeus
    /// --apply …`. a configuration that does not enable coeus is refused,
    /// since its coeusd would never answer
    pub async fn of(source: &StorePath, hostname: &Hostname) -> eyre::Result<Self> {
        todo!()
    }
}

/// whether something accepts connections on coeusd's port here
pub async fn listening(port: u16) -> bool {
    todo!()
}
