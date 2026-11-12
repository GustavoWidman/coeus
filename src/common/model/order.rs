use std::fmt;

use serde::{Deserialize, Serialize};

use super::{OrderId, Source};

/// what to do with the built system
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    /// activate now and make it the boot default
    #[default]
    Switch,
    /// make it the boot default without activating (NixOS only)
    Boot,
    /// activate now without touching the boot default (NixOS only)
    Test,
    /// build only, a preview with a closure diff
    Build,
}

impl Action {
    pub const fn activates(self) -> bool {
        matches!(self, Self::Switch | Self::Test)
    }

    /// whether the system profile gets a new generation
    pub const fn sets_profile(self) -> bool {
        matches!(self, Self::Switch | Self::Boot)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Switch => "switch",
            Self::Boot => "boot",
            Self::Test => "test",
            Self::Build => "build",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// the closed set of nix knobs an order may turn. anything else is the
/// daemon's configuration, never the client's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NixOptions {
    /// skip nix's evaluation cache
    pub no_eval_cache: bool,
    /// ignore stale substituters left by an older generation
    pub clean_substituters: bool,
}

/// who asked for a deployment
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Origin {
    /// someone at a terminal
    User { user: String, host: String },
    /// a GitHub Actions run
    GitHub {
        repository: String,
        workflow: String,
        run_id: u64,
        attempt: u32,
    },
    /// the daemon itself, catching up with the ref it follows
    Follow,
}

impl Origin {
    pub fn url(&self) -> Option<String> {
        match self {
            Self::GitHub {
                repository, run_id, ..
            } => Some(format!(
                "https://github.com/{repository}/actions/runs/{run_id}"
            )),
            _ => None,
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User { user, host } => write!(f, "{user}@{host}"),
            Self::GitHub {
                workflow, run_id, ..
            } => write!(f, "github {workflow} #{run_id}"),
            Self::Follow => f.write_str("catch-up"),
        }
    }
}

/// a request to bring one machine to a source
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub source: Source,
    pub action: Action,
    pub options: NixOptions,
    pub origin: Origin,
    /// replace the running job if it has not started activating
    pub now: bool,
}
