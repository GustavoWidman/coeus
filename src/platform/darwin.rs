use std::{fs::File, io::Read, process::Command};

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
        .unwrap_or_else(|| "/usr/bin/time".to_owned())
}

/// what to tell people when activation needs Full Disk Access it lacks
pub fn full_disk_access_missing(launcher: &str) -> String {
    format!(
        "{launcher} needs Full Disk Access to activate nix-darwin: add it in \
         System Settings > Privacy & Security > Full Disk Access"
    )
}
