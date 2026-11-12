//! `coeusd run <job-dir>`, the process that carries out one job: it fetches,
//! evaluates and builds the system, diffs it against the running one,
//! activates it and checks the machine is healthy, writing everything to the
//! job log. the service manager starts it, not coeusd, so it outlives
//! coeusd's restarts.

mod activation;
mod feed;
mod flake;
mod guard;
mod job;
mod nix;
mod process;

#[cfg(test)]
mod test;

use std::{fs, path::Path};

use eyre::Result;

use crate::platform::darwin;

pub use flake::Recipe;
pub use nix::{CLEAN_SUBSTITUTERS, CLEAN_TRUSTED_PUBLIC_KEYS};

/// carries out the job in `dir`. a job another runner holds, or one already
/// finished, is left alone.
pub async fn run(dir: &Path) -> Result<()> {
    job::Runner::run(dir).await
}

/// writes whether this process has Full Disk Access to `verdict`, as
/// `granted` or `missing`
pub async fn probe(verdict: &Path) -> Result<()> {
    let answer = match darwin::has_full_disk_access() {
        true => "granted",
        false => "missing",
    };

    let temporary = verdict.with_extension("tmp");
    fs::write(&temporary, answer)?;
    fs::rename(&temporary, verdict)?;
    Ok(())
}
