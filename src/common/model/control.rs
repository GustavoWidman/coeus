use serde::{Deserialize, Serialize};

use super::{Action, JobEvent, JobId, Platform, Stamped, StorePath, Timestamp};

/// how a machine took an order
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admission {
    pub job: JobId,
    pub placement: Placement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placement {
    /// the machine was idle, the job runs now
    Started,
    /// waits for the running job, replacing whatever waited before it
    Queued {
        behind: JobId,
        replaced: Option<JobId>,
    },
    /// replaced the running job, which had not started activating
    Preempted { replaced: JobId },
    /// the same order arrived before; this is the job it created then
    Duplicate,
}

/// why a machine refused an order
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rejection {
    /// macOS cannot activate without Full Disk Access for the launcher
    FullDiskAccessMissing { launcher: String },
    /// the action does not exist on this platform (boot or test on darwin)
    Unsupported { action: Action, platform: Platform },
    /// the uploaded snapshot this order names is not on the machine
    UnknownSnapshot,
    /// the generation to roll back to does not exist
    UnknownGeneration(u32),
    /// the daemon is restarting into a newer version
    Restarting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobSelector {
    /// the running job, else the queued one, else the newest finished one
    Current,
    Id(JobId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelReport {
    /// a queued job was withdrawn
    Withdrawn(JobId),
    /// the running job was asked to stop
    Stopping(JobId),
    /// the job is activating and has to finish
    TooLate(JobId),
    Nothing,
}

/// one piece of a job log, as attach streams it
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachFrame {
    /// one or more whole lines of nix's internal-json, ready for `nom --json`
    Nix(Vec<u8>),
    Event(Stamped<JobEvent>),
}

/// a generation of the system profile
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generation {
    pub number: u32,
    pub toplevel: StorePath,
    pub created_at: Timestamp,
    /// the running system
    pub current: bool,
    /// the system the machine booted
    pub booted: bool,
}
