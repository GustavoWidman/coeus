//! putting every secret on exactly its recipients: new ones encrypted from
//! memory, stale ones re-encrypted with the admin's identity

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use age::armor::{ArmoredWriter, Format};
use eyre::{WrapErr, bail, eyre};
use tempfile::NamedTempFile;

use super::{AdminIdentity, Header, Verdict};
use crate::bootstrap::{
    Blocker, SshKey,
    contract::{RepoPath, Rules},
};

/// every managed secret of the clone and how it stands
#[derive(Clone, Debug)]
pub struct Plan {
    root: PathBuf,
    pub files: Vec<Planned>,
    /// `.age` files the contract does not manage, left alone
    pub unmanaged: Vec<RepoPath>,
}

#[derive(Clone, Debug)]
pub struct Planned {
    pub path: RepoPath,
    pub recipients: Vec<SshKey>,
    pub header: Option<Header>,
    pub verdict: Verdict,
}

/// reads the header of every file `rules` names, under `root`; no key needed
pub fn plan(root: &Path, rules: &Rules) -> eyre::Result<Plan> {
    let mut files = Vec::new();
    let mut unmanaged = Vec::new();
    for (path, recipients) in rules {
        let Some(recipients) = recipients else {
            unmanaged.push(path.clone());
            continue;
        };
        let (header, verdict) = match Header::read(&root.join(path.as_str())) {
            Ok(header) => {
                let verdict = header.verdict(recipients);
                (Some(header), verdict)
            }
            Err(error) => (None, Verdict::Unreadable(format!("{error:#}"))),
        };
        files.push(Planned {
            path: path.clone(),
            recipients: recipients.clone(),
            header,
            verdict,
        });
    }
    Ok(Plan {
        root: root.to_owned(),
        files,
        unmanaged,
    })
}

impl Plan {
    /// the files whose header disagrees with their recipients
    pub fn stale(&self) -> impl Iterator<Item = &Planned> {
        self.files
            .iter()
            .filter(|file| matches!(file.verdict, Verdict::Stale { .. }))
    }

    /// the stale files whose header does not name `admin`, which it could
    /// not decrypt: checked before any sudo prompt
    pub fn unopenable_by(&self, admin: &SshKey) -> Vec<RepoPath> {
        self.stale()
            .filter(|file| {
                !file
                    .header
                    .as_ref()
                    .is_some_and(|header| header.names(admin))
            })
            .map(|file| file.path.clone())
            .collect()
    }

    /// refuses, naming the files, when `admin` cannot open every stale one:
    /// this machine is not an admin of the fleet, and a sudo prompt would
    /// only lead to a failure later
    pub fn require_admin(&self, admin: &SshKey) -> Result<(), Blocker> {
        let files = self.unopenable_by(admin);
        if files.is_empty() {
            Ok(())
        } else {
            Err(Blocker::NotAdmin { files })
        }
    }

    /// re-encrypts each stale file for its recipients, keeping armor as it
    /// was. a run stopped between two files leaves each one old or new,
    /// never half, and the next run rekeys the rest. returns what it rewrote.
    pub fn apply(
        &self,
        identity: &AdminIdentity,
        mut progress: impl FnMut(usize, usize, &RepoPath),
    ) -> eyre::Result<Vec<RepoPath>> {
        let unopenable = self.unopenable_by(identity.key());
        if !unopenable.is_empty() {
            bail!(
                "{} cannot open {}",
                identity.key().fingerprint(),
                list(&unopenable)
            );
        }

        let stale: Vec<&Planned> = self.stale().collect();
        let mut rewritten = Vec::with_capacity(stale.len());
        for (index, file) in stale.iter().enumerate() {
            progress(index, stale.len(), &file.path);
            let armored = file.header.as_ref().is_some_and(|header| header.armored);
            let on_disk = self.root.join(file.path.as_str());
            let rekey = || -> eyre::Result<()> {
                let ciphertext = fs::read(&on_disk)?;
                let plaintext = identity.decrypt(&ciphertext)?;
                replace(&on_disk, &encrypt(&plaintext, &file.recipients, armored)?)
            };
            rekey().wrap_err_with(|| format!("failed to rekey {}", file.path))?;
            rewritten.push(file.path.clone());
        }
        Ok(rewritten)
    }
}

/// encrypts a new secret to `recipients` and puts it at `file`, binary. a
/// file already there is never overwritten
pub fn create(file: &Path, plaintext: &[u8], recipients: &[SshKey]) -> eyre::Result<()> {
    let ciphertext = encrypt(plaintext, recipients, false)?;
    let temporary = sibling(file, &ciphertext)?;
    temporary.persist_noclobber(file).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            eyre!("{} exists already", file.display())
        } else {
            eyre!(error.error).wrap_err(format!("failed to write {}", file.display()))
        }
    })?;
    sync_parent(file)
}

/// `plaintext` encrypted to each of `recipients` once
fn encrypt(plaintext: &[u8], recipients: &[SshKey], armored: bool) -> eyre::Result<Vec<u8>> {
    let recipients = recipients
        .iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|key| {
            key.to_string()
                .parse::<age::ssh::Recipient>()
                .map_err(|error| eyre!("age cannot encrypt to {key}: {error:?}"))
        })
        .collect::<eyre::Result<Vec<_>>>()?;
    if recipients.is_empty() {
        bail!("a secret needs at least one recipient");
    }
    let encryptor = age::Encryptor::with_recipients(
        recipients
            .iter()
            .map(|recipient| recipient as &dyn age::Recipient),
    )?;

    let format = if armored {
        Format::AsciiArmor
    } else {
        Format::Binary
    };
    let mut ciphertext = Vec::new();
    let mut writer = encryptor.wrap_output(ArmoredWriter::wrap_output(&mut ciphertext, format)?)?;
    writer.write_all(plaintext)?;
    writer.finish()?.finish()?;
    Ok(ciphertext)
}

/// writes through a temporary sibling, fsync and a rename
fn replace(file: &Path, bytes: &[u8]) -> eyre::Result<()> {
    let temporary = sibling(file, bytes)?;
    // git keeps the mode, so the new file keeps the old one's
    let permissions = fs::metadata(file)?.permissions();
    temporary.as_file().set_permissions(permissions)?;
    temporary.persist(file).map_err(|error| {
        eyre!(error.error).wrap_err(format!("failed to replace {}", file.display()))
    })?;
    sync_parent(file)
}

/// `bytes` in a new file beside `file`, on disk
fn sibling(file: &Path, bytes: &[u8]) -> eyre::Result<NamedTempFile> {
    let directory = parent(file);
    let name = file
        .file_name()
        .ok_or_else(|| eyre!("{} names no file", file.display()))?;
    let mut prefix = std::ffi::OsString::from(".");
    prefix.push(name);
    prefix.push(".");
    let mut temporary = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".tmp")
        .tempfile_in(directory)
        .wrap_err_with(|| format!("failed to write in {}", directory.display()))?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    Ok(temporary)
}

/// makes a rename durable
fn sync_parent(file: &Path) -> eyre::Result<()> {
    File::open(parent(file))?.sync_all()?;
    Ok(())
}

fn parent(file: &Path) -> &Path {
    match file.parent() {
        Some(directory) if !directory.as_os_str().is_empty() => directory,
        _ => Path::new("."),
    }
}

fn list(paths: &[RepoPath]) -> String {
    paths
        .iter()
        .map(RepoPath::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}
