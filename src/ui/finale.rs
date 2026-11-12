//! the lines after the checklist settles: the verdict, then what is left for
//! a person, from the evidence only:
//!
//! - merge the pull request
//! - `coeus reboot <host>` when the reboot was declined
//! - on a Mac whose coeusd lacks Full Disk Access, `coeus doctor --fix` on
//!   that Mac
//! - the contract's notes
//! - for a failure, what failed, its last lines, and `--retry` or the file
//!   to fix; for a block, what the person must do, and that running again
//!   picks up from there

// the bootstrap's ui unit fills this module in; until then its
// signatures have unused parameters
#![allow(unused_variables)]

use crate::bootstrap::Finale;

pub fn lines(finale: &Finale) -> Vec<String> {
    todo!()
}
