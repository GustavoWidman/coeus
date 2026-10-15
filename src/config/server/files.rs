//! Canonical-path checks plus descriptor validation. Runtime secret symlinks are
//! supported; parent directories and same-UID writers remain a trusted boundary.
use super::{ConfigInner, decode_key, parse};
use eyre::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub(super) fn load(path: &Path, uid: u32) -> Result<(PathBuf, ConfigInner)> {
    load_with_store(path, uid, Path::new("/nix/store"), 0)
}

// The explicit store parameters allow unprivileged fixtures to exercise the same
// policy. Production always uses /nix/store and root, never an environment override.
pub(super) fn load_with_store(
    path: &Path,
    uid: u32,
    store: &Path,
    store_uid: u32,
) -> Result<(PathBuf, ConfigInner)> {
    let (canonical, file) = open_regular(path, "Coeus service config")?;
    let metadata = file
        .metadata()
        .wrap_err("cannot inspect Coeus service config")?;
    let public = canonical.starts_with(store);
    if public {
        ensure!(
            metadata.uid() == store_uid && metadata.mode() & 0o7222 == 0,
            "public store config must be root-owned, non-writable, and without special bits"
        );
    } else {
        validate_private(&metadata, uid, "Coeus service config")?;
    }
    let bytes = bounded_read(file, 65536, "server config")?;
    let raw =
        std::str::from_utf8(&bytes).map_err(|_| eyre::eyre!("invalid server TOML encoding"))?;
    let settings = parse(raw)?;
    let key = match (settings.key, settings.key_file) {
        (Some(inline), None) => {
            ensure!(
                !public,
                "public store config must use key_file; inline PSK is forbidden"
            );
            decode_key(inline.as_bytes())?
        }
        (None, Some(path)) => {
            ensure!(
                path.is_absolute(),
                "key_file must be an absolute runtime path"
            );
            let (canonical, file) = open_regular(&path, "Coeus key file")?;
            ensure!(
                !canonical.starts_with(store),
                "key_file must resolve outside the Nix store"
            );
            validate_private(
                &file.metadata().wrap_err("cannot inspect Coeus key file")?,
                uid,
                "Coeus key file",
            )?;
            let bytes = bounded_read(file, 65, "key file")?;
            // Exactly 64 ASCII hex characters, optionally followed by one LF.
            let bytes = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
            decode_key(bytes)?
        }
        _ => {
            eyre::bail!("server settings require exactly one of key or key_file");
        }
    };
    Ok((
        canonical,
        ConfigInner {
            key,
            host: settings.host,
            port: settings.port,
            repo_url: settings.repo_url,
            state_directory: settings.state_directory,
        },
    ))
}

fn open_regular(path: &Path, label: &str) -> Result<(PathBuf, File)> {
    let canonical = fs::canonicalize(path).wrap_err_with(|| format!("cannot resolve {label}"))?;
    Ok((canonical.clone(), open_resolved(&canonical, label)?))
}

pub(super) fn open_resolved(canonical: &Path, label: &str) -> Result<File> {
    // Do not block on FIFOs or follow a replaced final symlink after resolution.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((nix::fcntl::OFlag::O_NONBLOCK | nix::fcntl::OFlag::O_NOFOLLOW).bits())
        .open(canonical)
        .wrap_err_with(|| format!("cannot read {label}"))?;
    ensure!(file.metadata()?.is_file(), "{label} must be a regular file");
    Ok(file)
}

fn validate_private(metadata: &fs::Metadata, uid: u32, label: &str) -> Result<()> {
    ensure!(
        metadata.uid() == uid,
        "{label} must be owned by the service user"
    );
    ensure!(
        private_mode(metadata.mode()),
        "{label} mode must be 0600 or 0400"
    );
    Ok(())
}

fn bounded_read(file: File, limit: u64, label: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .wrap_err_with(|| format!("cannot read {label}"))?;
    ensure!(bytes.len() as u64 <= limit, "{label} exceeds size limit");
    Ok(bytes)
}

pub(super) fn private_mode(mode: u32) -> bool {
    matches!(mode & 0o7777, 0o600 | 0o400)
}
