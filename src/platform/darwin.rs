use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
};

use eyre::{Result, WrapErr};
use log::info;

/// the system TCC database: only a process with Full Disk Access can read it
const TCC_DB: &str = "/Library/Application Support/com.apple.TCC/TCC.db";

/// whether this process inherited Full Disk Access from the program that
/// launched it, the same grant nix-darwin's activation needs
pub fn has_full_disk_access() -> bool {
    File::open(TCC_DB).is_ok_and(|mut db| db.read(&mut [0; 1]).is_ok())
}

/// the program this process runs under, the one Full Disk Access is granted to
pub fn launcher() -> String {
    let parent = rustix::process::getppid().map_or(0, |pid| pid.as_raw_nonzero().get());

    Command::new("/bin/ps")
        .args(["-o", "comm=", "-p", &parent.to_string()])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "coeus-launcher".to_owned())
}

/// what to tell people when activation needs Full Disk Access it lacks
pub fn full_disk_access_missing(launcher: &str) -> String {
    format!(
        "nix-darwin needs Full Disk Access to create or delete users: grant it to \
         {launcher} in System Settings > Privacy & Security > Full Disk Access, \
         or run `coeus doctor --fix` on this Mac"
    )
}

/// puts the launcher that ships next to `exe` at `target`, unless one is
/// already there. never replacing it keeps its code identity, and so the Full
/// Disk Access granted to it, stable across coeus updates.
pub fn install_launcher(target: &Path, exe: &Path) -> Result<()> {
    if target.exists() {
        return Ok(());
    }

    let shipped = exe.with_file_name("coeus-launcher");
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    let staging = target.with_extension("new");
    fs::copy(&shipped, &staging)
        .wrap_err_with(|| format!("failed to copy {}", shipped.display()))?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))?;
    fs::rename(&staging, target)?;

    info!("installed the runner launcher at {}", target.display());
    Ok(())
}
