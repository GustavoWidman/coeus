//! bringing a machine that already runs NixOS, or a Mac, into the fleet: it
//! is enrolled in the configuration repository, the secrets are rekeyed for
//! it, it switches to its fleet configuration with the coeus that
//! configuration pins, its coeusd answers over the tailnet, and a pull
//! request adds it.
//!
//! a run never trusts a record of its own progress. every step looks at the
//! world (the branch, the secrets' headers, the machine, the forge), acts
//! only on the difference, and looks again. a run stopped anywhere is picked
//! up by running it again.

// the units of the bootstrap fill these modules in; until then most of the
// tree is signatures with unused parameters
#![allow(dead_code, unused_variables)]

pub mod branch;
pub mod contract;
pub mod fleet;
pub mod key;
pub mod remote;
pub mod run;
pub mod secrets;
pub mod state;
pub mod step;
pub mod switch;

#[cfg(test)]
mod test;

use std::{collections::BTreeMap, path::PathBuf};

pub use key::SshKey;
pub use run::run;
pub use step::{Blocker, Evidence, Finale, Step};

use crate::{common::model::Hostname, config::ClientSettings};

/// what a run starts from: the arguments, the client settings, and what
/// earlier runs saved for this machine
pub struct Bootstrap {
    pub host: Hostname,
    pub settings: ClientSettings,
    pub state: state::StateDir,
    /// the saved inputs, with this run's arguments applied over them
    pub saved: state::Saved,
    pub options: Options,
}

/// what this run was asked beyond the saved inputs
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// the identity that opens every secret; the contract's when unset
    pub identity: Option<PathBuf>,
    /// trust the host key with this `SHA256:…` fingerprint without asking
    pub host_key: Option<String>,
    /// order the first switch of this head again after it failed
    pub retry: bool,
    /// `--answer id=value`, checked against the questions once they are known
    pub answers: BTreeMap<String, String>,
}
