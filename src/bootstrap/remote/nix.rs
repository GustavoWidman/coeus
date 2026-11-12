//! a Mac before nix-darwin's first activation: Determinate Nix installed,
//! and the shell files an installer edited moved out of nix-darwin's way

use super::Stock;
use crate::cli::Console;

/// nix on the machine, as the probe found it
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NixState {
    Installed {
        version: String,
    },
    /// the installer's receipt is there but nix does not answer: an install
    /// a crash cut short
    Broken,
    Missing,
}

/// installs when missing, `repair`s when broken (reinstalling when that
/// fails); each is one root call whose last output line shows in the row.
/// returns the version that answers afterwards.
pub async fn converge(stock: &Stock, console: &Console) -> eyre::Result<String> {
    todo!()
}

/// `/etc/bashrc`, `/etc/zshrc` and `/etc/zprofile` moved to
/// `.before-nix-darwin` when nix-darwin did not write them; the ones moved,
/// for a step note
pub async fn move_etc_aside(stock: &Stock) -> eyre::Result<Vec<String>> {
    todo!()
}
