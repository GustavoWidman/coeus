use std::fmt;

use serde::{Deserialize, Serialize};

use super::{Hostname, JobId, JobSummary, JobView, Pinned, Source, StorePath, Timestamp};

/// everything the live views show about one machine. streamed whole, latest
/// value wins, so a watcher that joins late or falls behind needs no replay.
/// bounded so it always fits one packet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineView {
    pub host: HostInfo,
    pub system: SystemInfo,
    pub daemon: DaemonState,
    pub running: Option<JobView>,
    pub next: Option<JobSummary>,
    /// finished jobs, newest first, at most [MachineView::RECENT]
    pub recent: Vec<JobSummary>,
}

impl MachineView {
    pub const RECENT: usize = 8;

    /// finds a job wherever it currently sits on this machine
    pub fn job(&self, id: JobId) -> Option<&JobSummary> {
        self.running
            .iter()
            .map(|view| &view.summary)
            .chain(&self.next)
            .chain(&self.recent)
            .find(|summary| summary.id == id)
    }

    /// the newest job this machine knows of
    pub fn latest(&self) -> Option<&JobSummary> {
        self.next
            .iter()
            .chain(self.running.iter().map(|view| &view.summary))
            .chain(&self.recent)
            .next()
    }
}

/// fixed for a daemon's lifetime, also sent in the hello
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: Hostname,
    pub platform: Platform,
    /// `aarch64-darwin`, `x86_64-linux`, ...
    pub system: String,
    pub version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    NixOs,
    Darwin,
}

impl Platform {
    pub const fn current() -> Self {
        match cfg!(target_os = "macos") {
            true => Self::Darwin,
            false => Self::NixOs,
        }
    }

    /// the flake output that holds this platform's systems
    pub const fn configurations(self) -> &'static str {
        match self {
            Self::NixOs => "nixosConfigurations",
            Self::Darwin => "darwinConfigurations",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NixOs => "nixos",
            Self::Darwin => "darwin",
        })
    }
}

/// facts read from the machine itself, refreshed after every job and every
/// minute, so changes made outside coeus show up too
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    /// `/run/current-system`
    pub current: Option<StorePath>,
    /// the system profile's generation number
    pub generation: Option<u32>,
    /// the profile differs from the running system (after a `boot` deploy)
    pub pending_boot: bool,
    /// the running kernel differs from the current system's
    pub reboot_required: bool,
    /// the source of the job that produced the running system, if one did
    pub deployed: Option<Source>,
    pub follow: Option<Follow>,
    pub fda: FdaStatus,
    pub checked_at: Timestamp,
}

/// the ref a machine catches up with on its own
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Follow {
    pub git_ref: String,
    /// where the ref pointed when last checked
    pub latest: Option<Pinned>,
    /// a deploy of something else pauses catching up until the ref is deployed again
    pub paused: bool,
}

/// Full Disk Access for the stable parent macOS runners start under
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FdaStatus {
    Granted,
    Missing,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaemonState {
    Serving,
    /// draining before exiting into a newer coeusd
    Restarting,
}
