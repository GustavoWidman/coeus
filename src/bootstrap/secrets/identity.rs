//! the identity an admin opens every secret with, by default its ssh host
//! key. its public half is world-readable and checked first; the private
//! half is read once, through `sudo cat` when only root may read it (sudo
//! prompts on the terminal itself, so Touch ID and the person's pam setup
//! apply), and lives in memory only.

use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
};

use eyre::{WrapErr, bail, eyre};
use tokio::process::Command;
use zeroize::Zeroizing;

use crate::{bootstrap::SshKey, utils::secret};

pub struct AdminIdentity {
    public: SshKey,
    /// age keeps the private key in a secret box, zeroed on drop
    identity: age::ssh::Identity,
}

impl AdminIdentity {
    /// `<path>.pub`, no privilege needed
    pub fn public(path: &Path) -> eyre::Result<SshKey> {
        let file = public_path(path);
        let text = std::fs::read_to_string(&file).wrap_err_with(|| {
            format!("failed to read the admin's public key {}", file.display())
        })?;
        SshKey::parse(text.trim()).wrap_err_with(|| format!("in {}", file.display()))
    }

    /// the private key at `path`, read directly when this user may, through
    /// `sudo cat` otherwise; the caller hands the terminal to sudo
    pub async fn unlock(path: &Path) -> eyre::Result<Self> {
        let public = Self::public(path)?;
        let private = match std::fs::read(path) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => sudo_cat(path).await?,
            Err(error) => {
                return Err(error).wrap_err_with(|| {
                    format!("failed to read the admin's identity {}", path.display())
                });
            }
        };
        Self::from_private(public, &private)
            .wrap_err_with(|| format!("in the admin's identity {}", path.display()))
    }

    /// an unencrypted OpenSSH private key, whose public half must be `public`
    fn from_private(public: SshKey, private: &[u8]) -> eyre::Result<Self> {
        let identity = secret::ssh_identity(private)?;
        let found = age::ssh::Recipient::try_from(identity.clone())
            .map_err(|_| eyre!("an ssh-ed25519 or ssh-rsa key is needed"))?;
        if SshKey::parse(&found.to_string())? != public {
            bail!("the private key is not the one its .pub file names ({public})");
        }
        Ok(Self { public, identity })
    }

    pub fn key(&self) -> &SshKey {
        &self.public
    }

    /// the plaintext of an age file, binary or armored
    pub fn decrypt(&self, ciphertext: &[u8]) -> eyre::Result<Zeroizing<Vec<u8>>> {
        secret::decrypt(&self.identity, ciphertext)
    }
}

fn public_path(path: &Path) -> PathBuf {
    let mut file = path.as_os_str().to_owned();
    file.push(".pub");
    PathBuf::from(file)
}

/// `sudo cat <path>`, the terminal inherited so sudo can prompt
async fn sudo_cat(path: &Path) -> eyre::Result<Zeroizing<Vec<u8>>> {
    let output = Command::new("sudo")
        .arg("cat")
        .arg("--")
        .arg(path)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .await
        .wrap_err("failed to run sudo")?;
    let private = Zeroizing::new(output.stdout);
    if !output.status.success() {
        bail!("`sudo cat {}` failed ({})", path.display(), output.status);
    }
    Ok(private)
}
