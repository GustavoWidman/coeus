//! files only this admin's user may read: the pin and the session key

use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::Path};

use eyre::{Result, WrapErr};

/// `bytes` at `path`, 0600, through a temporary sibling, fsync and a rename,
/// so a crash leaves the old file or the new one
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension("coeus-new");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .wrap_err_with(|| format!("could not write {}", temporary.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)
        .wrap_err_with(|| format!("could not write {}", path.display()))?;
    Ok(())
}
