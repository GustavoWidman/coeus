use serde::{Deserialize, Serialize};

use crate::common::{
    model::{AttachFrame, JobId, JobSelector, JobSummary, MachineView, Outcome},
    proto::Packet,
};

/// ask for the machine's view; with `follow`, keep receiving it as it changes
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct WatchRequest {
    pub follow: bool,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct MachineUpdate {
    pub view: MachineView,
}

/// stream a job's log from a byte offset, live until the job finishes
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachRequest {
    pub job: JobSelector,
    pub from: u64,
    /// have the machine draw nix-output-monitor's view of the whole log at
    /// this size and send [AttachScreen]s; without it, or when the machine
    /// cannot draw, the log comes as [AttachBatch]es of frames
    pub terminal: Option<TerminalSize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub columns: u16,
    pub rows: u16,
}

/// terminal output of a drawn attach, for the client to write as is. the
/// job's events still come as batches, so the client knows how it ended
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachScreen {
    pub bytes: Vec<u8>,
}

impl AttachScreen {
    /// the most one carries, well within an encrypted frame
    pub const MAX: usize = 32 * 1024;
}

/// the client's terminal changed size during a drawn attach
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachResize {
    pub size: TerminalSize,
}

/// the job the attach resolved to, sent once before any batch
#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachStarted {
    pub job: JobSummary,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachBatch {
    pub frames: Vec<AttachFrame>,
    /// the log offset after the last frame, where a reconnect resumes
    pub end: u64,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct AttachEnded {
    pub job: JobId,
    pub outcome: Outcome,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct HistoryRequest {
    pub limit: u32,
}

#[derive(Debug, Serialize, Deserialize, Packet)]
pub struct HistoryResponse {
    /// newest first
    pub jobs: Vec<JobSummary>,
}
