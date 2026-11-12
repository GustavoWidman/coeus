//! how commands become root on the machine. before the session key is
//! installed, a stock user needs sudo, maybe with a password: asked once per
//! run in the checklist, kept in memory, written to `sudo -S` on each call
//! and never saved.

use zeroize::Zeroizing;

use super::{Call, Ssh};
use crate::cli::Console;

pub enum Privilege {
    /// root already: the session key, or a root login
    Root,
    /// `sudo -n` works
    Sudo,
    /// `sudo -S -p ''`, fed this password
    SudoPassword(Zeroizing<String>),
}

impl Privilege {
    /// finds out which, asking for the password only when sudo needs one,
    /// and checking it with `sudo -S -v` before keeping it
    pub async fn establish(ssh: &Ssh, login: &str, console: &Console) -> eyre::Result<Self> {
        todo!()
    }

    /// `call`'s argv as root, and the line to write first on its stdin
    pub fn wrap(&self, call: &Call) -> (Vec<String>, Option<&str>) {
        todo!()
    }
}
