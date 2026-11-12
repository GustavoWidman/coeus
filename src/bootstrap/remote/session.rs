//! the per-machine session key: an ed25519 key made on the admin, kept 0600
//! in the state directory, and put in root's `authorized_keys` by the first
//! privileged act. every later ssh is `root@` with it, so a run survives the
//! stock user vanishing in the first switch, or a rollback that loses its
//! password hash. the cleanup step and `--forget` take it out again.
//!
//! its `authorized_keys` line carries the comment `coeus-bootstrap@<host>`,
//! so a person reading the file knows whose it is.

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, eyre};
use ssh_key::{Algorithm, LineEnding, PrivateKey, rand_core::OsRng};

use super::{Call, Privilege, Ssh, Stock, exec, private};
use crate::{
    bootstrap::{SshKey, state::StateDir},
    common::model::Hostname,
};

pub struct SessionKey {
    private: PathBuf,
    public: SshKey,
    host: Hostname,
}

impl SessionKey {
    /// the key in the state directory, made on first use
    pub fn ensure(state: &StateDir) -> Result<Self> {
        if let Some(key) = Self::existing(state)? {
            return Ok(key);
        }

        let comment = format!("coeus-bootstrap@{}", state.host());
        let mut key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
            .map_err(|error| eyre!("could not make the session key: {error}"))?;
        key.set_comment(&comment);
        let pem = key
            .to_openssh(LineEnding::LF)
            .map_err(|error| eyre!("could not encode the session key: {error}"))?;
        let public = key
            .public_key()
            .to_openssh()
            .map_err(|error| eyre!("could not encode the session key: {error}"))?;

        // the public half first: a private key without it is made again
        let path = state.session_key();
        private::write(&public_path(&path), format!("{public}\n").as_bytes())?;
        private::write(&path, pem.as_bytes())?;
        Self::existing(state)?.ok_or_else(|| eyre!("the session key vanished once written"))
    }

    /// the key if a run made one, for cleanup and `--forget`
    pub fn existing(state: &StateDir) -> Result<Option<Self>> {
        let path = state.session_key();
        if !path.exists() {
            return Ok(None);
        }
        let public = std::fs::read_to_string(public_path(&path))
            .wrap_err("the session key's public half is missing")?;
        let public = SshKey::parse(&public)?;
        Ok(Some(Self {
            private: path,
            public,
            host: state.host().clone(),
        }))
    }

    pub fn private(&self) -> &Path {
        &self.private
    }

    pub fn public(&self) -> &SshKey {
        &self.public
    }

    /// in root's `authorized_keys` unless there already, through the login
    /// and its privilege
    pub async fn install(&self, ssh: &Ssh, privilege: &Privilege) -> Result<()> {
        let call = Call::InstallSessionKey {
            key: self.public.clone(),
            host: self.host.clone(),
        };
        exec::text(ssh, privilege, &call)
            .await
            .wrap_err("could not install the session key for root")?;
        Ok(())
    }

    /// out of root's `authorized_keys`; absent already is done
    pub async fn remove(&self, stock: &Stock) -> Result<()> {
        let call = Call::RemoveSessionKey {
            key: self.public.clone(),
        };
        exec::text(&stock.ssh, &stock.privilege, &call)
            .await
            .wrap_err("could not remove the session key from root's authorized_keys")?;
        Ok(())
    }
}

fn public_path(private: &Path) -> PathBuf {
    let mut name = private.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}
